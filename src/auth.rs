//! Accounts: offline profiles and Microsoft accounts via the OAuth device-code flow.
//!
//! Microsoft login needs an Azure application ID ("client ID") that is allowed to use
//! the Minecraft Services API; see README for how to register one.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::http::{agent, read_json};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AccountKind {
    Offline,
    Microsoft,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub kind: AccountKind,
    pub name: String,
    /// UUID without dashes, as Minecraft expects.
    pub uuid: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub xuid: Option<String>,
    /// Unix time (seconds) when `access_token` expires.
    #[serde(default)]
    pub expires_at: u64,
}

impl Account {
    pub fn offline(name: &str) -> Account {
        // Same as Java's UUID.nameUUIDFromBytes("OfflinePlayer:" + name), used by servers in offline mode.
        let hash: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
        let uuid = uuid::Builder::from_md5_bytes(hash).into_uuid();
        Account {
            kind: AccountKind::Offline,
            name: name.to_string(),
            uuid: uuid.simple().to_string(),
            access_token: "0".into(),
            refresh_token: None,
            xuid: None,
            expires_at: 0,
        }
    }

    pub fn user_type(&self) -> &'static str {
        match self.kind {
            AccountKind::Offline => "legacy",
            AccountKind::Microsoft => "msa",
        }
    }

    pub fn needs_refresh(&self) -> bool {
        self.kind == AccountKind::Microsoft && now() + 300 >= self.expires_at
    }
}

pub fn valid_offline_name(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Clone, Debug)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    device_code: String,
    interval: u64,
    expires_in: u64,
}

const MS_DEVICE_CODE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const MS_TOKEN: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const SCOPE: &str = "XboxLive.signin offline_access";

pub fn request_device_code(client_id: &str) -> Result<DeviceCode> {
    if client_id.trim().is_empty() {
        bail!("Azure application client ID is not set (`ms_client_id` in the settings)");
    }
    let resp = agent().post(MS_DEVICE_CODE).send_form([("client_id", client_id.trim()), ("scope", SCOPE)])?;
    let v = read_json(resp, "Microsoft device code")?;
    Ok(DeviceCode {
        user_code: str_field(&v, "user_code")?,
        verification_uri: str_field(&v, "verification_uri")?,
        device_code: str_field(&v, "device_code")?,
        interval: v["interval"].as_u64().unwrap_or(5),
        expires_in: v["expires_in"].as_u64().unwrap_or(900),
    })
}

/// Blocks until the user finishes signing in (or the code expires).
pub fn complete_device_code(client_id: &str, code: &DeviceCode) -> Result<Account> {
    let deadline = now() + code.expires_in;
    let mut interval = code.interval;
    loop {
        std::thread::sleep(Duration::from_secs(interval));
        if now() > deadline {
            bail!("sign-in timed out");
        }
        let resp = agent().post(MS_TOKEN).send_form([
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id.trim()),
            ("device_code", code.device_code.as_str()),
        ])?;
        let status = resp.status();
        let mut resp = resp;
        let v: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        if status.is_success() {
            return finish_login(&str_field(&v, "access_token")?, v["refresh_token"].as_str());
        }
        match v["error"].as_str() {
            Some("authorization_pending") => continue,
            Some("slow_down") => interval += 5,
            Some("authorization_declined") => bail!("sign-in was declined by the user"),
            Some("expired_token") => bail!("the code has expired, try again"),
            _ => bail!("Microsoft sign-in failed: {}", v["error_description"].as_str().unwrap_or("?")),
        }
    }
}

/// Refreshes an expired Microsoft account using its refresh token.
pub fn refresh(client_id: &str, account: &Account) -> Result<Account> {
    let token = account.refresh_token.as_deref().context("no refresh token, sign in again")?;
    let resp = agent().post(MS_TOKEN).send_form([
        ("grant_type", "refresh_token"),
        ("client_id", client_id.trim()),
        ("refresh_token", token),
        ("scope", SCOPE),
    ])?;
    let v = read_json(resp, "Microsoft token refresh")?;
    finish_login(&str_field(&v, "access_token")?, v["refresh_token"].as_str().or(Some(token)))
}

/// Microsoft token -> Xbox Live -> XSTS -> Minecraft token -> profile.
fn finish_login(ms_token: &str, refresh_token: Option<&str>) -> Result<Account> {
    let xbl = read_json(
        agent().post("https://user.auth.xboxlive.com/user/authenticate").send_json(json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={ms_token}"),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        }))?,
        "Xbox Live",
    )?;
    let xbl_token = str_field(&xbl, "Token")?;

    let resp = agent().post("https://xsts.auth.xboxlive.com/xsts/authorize").send_json(json!({
        "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT",
    }))?;
    if resp.status() == 401 {
        let mut resp = resp;
        let v: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
        bail!(match v["XErr"].as_u64() {
            Some(2148916233) => "this Microsoft account has no Xbox profile; create one at xbox.com",
            Some(2148916235) => "Xbox Live is not available in your country",
            Some(2148916236 | 2148916237) => "the account needs age verification at xbox.com",
            Some(2148916238) => "child account: it must be added to a Microsoft family group",
            _ => "XSTS authorization was denied",
        });
    }
    let xsts = read_json(resp, "XSTS")?;
    let xsts_token = str_field(&xsts, "Token")?;
    let claims = &xsts["DisplayClaims"]["xui"][0];
    let uhs = claims["uhs"].as_str().context("XSTS response has no uhs")?;

    let mc = read_json(
        agent()
            .post("https://api.minecraftservices.com/authentication/login_with_xbox")
            .send_json(json!({ "identityToken": format!("XBL3.0 x={uhs};{xsts_token}") }))?,
        "Minecraft Services",
    )
    .context("Minecraft Services rejected the login (the client ID must be approved by Mojang)")?;
    let mc_token = str_field(&mc, "access_token")?;

    let resp = agent()
        .get("https://api.minecraftservices.com/minecraft/profile")
        .header("Authorization", &format!("Bearer {mc_token}"))
        .call()?;
    if resp.status() == 404 {
        bail!("this account does not own Minecraft: Java Edition");
    }
    let profile = read_json(resp, "Minecraft profile")?;

    Ok(Account {
        kind: AccountKind::Microsoft,
        name: str_field(&profile, "name")?,
        uuid: str_field(&profile, "id")?,
        access_token: mc_token,
        refresh_token: refresh_token.map(String::from),
        xuid: claims["xid"].as_str().map(String::from),
        expires_at: now() + mc["expires_in"].as_u64().unwrap_or(86400),
    })
}

fn str_field(v: &Value, key: &str) -> Result<String> {
    v[key].as_str().map(String::from).with_context(|| format!("response has no `{key}` field"))
}
