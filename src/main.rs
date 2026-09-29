//! Command-line front end for the launcher core.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Result, bail};
use gplauncher::auth::{self, Account};
use gplauncher::instance::{self, Instance};
use gplauncher::settings::Settings;
use gplauncher::{Event, Reporter, import, launch, version};

const USAGE: &str = "\
Usage:
  gplauncher launch [INSTANCE|VERSION] [--user NAME]
      Launch an instance, or a plain version from the game folder. Defaults to the latest release.
      Plays as the account selected in the launcher, or offline as NAME with --user.
  gplauncher import FILE...
      Import .mrpack (Modrinth), CurseForge .zip or MultiMC/Prism .zip modpacks as new instances.
      CurseForge packs need an API key in CURSEFORGE_API_KEY or the launcher settings.
  gplauncher versions [--all]
      List available versions (releases only, unless --all).
  gplauncher instances
      List instances.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("launch") => cmd_launch(&args[1..]),
        Some("import") => cmd_import(&args[1..]),
        Some("versions") => cmd_versions(&args[1..]),
        Some("instances") => cmd_instances(),
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Prints status lines and game output; progress is ignored.
fn stdout_reporter() -> Reporter {
    Reporter::new(|event| match event {
        Event::Status(s) => println!("[*] {s}"),
        Event::Log(s) => println!("{s}"),
        Event::GameExited(code) => println!("[*] game exited: {code:?}"),
        _ => {}
    })
}

fn cmd_launch(args: &[String]) -> Result<()> {
    let mut target = None;
    let mut user = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--user" => match it.next() {
                Some(name) => user = Some(name.clone()),
                None => bail!("--user needs a value"),
            },
            v => target = Some(v.to_string()),
        }
    }
    if let Some(user) = &user
        && !auth::valid_offline_name(user)
    {
        bail!("invalid player name {user:?}: use 3-16 characters, letters, digits and `_`");
    }

    let mut settings = Settings::load();
    let manifest = version::list(&settings.data_dir)?;
    let target = match target {
        Some(t) => t,
        None => manifest.iter().find(|v| v.kind == "release").map(|v| v.id.clone()).unwrap_or_default(),
    };
    let mut instance = instance::list(&settings.data_dir)
        .into_iter()
        .find(|i| i.name == target || i.id == target)
        .unwrap_or_else(|| Instance::ephemeral(&target, settings.game_dir()));
    if user.is_some() || settings.account().is_none() {
        settings.accounts = vec![Account::offline(user.as_deref().unwrap_or("Player"))];
        settings.selected_account = 0;
    }
    let stdout = stdout_reporter();
    let reporter = Reporter::new(move |event| match event {
        Event::AccountRefreshed(account) => save_account(account),
        event => stdout.send(event),
    });
    launch::run(&settings, &mut instance, &manifest, &reporter, &launch::GameHandle::default())
}

/// Stores a refreshed Microsoft account back into the launcher settings.
fn save_account(account: Account) {
    let mut settings = Settings::load();
    if let Some(slot) =
        settings.accounts.iter_mut().find(|a| a.kind == account.kind && a.uuid == account.uuid)
    {
        *slot = account;
        if let Err(e) = settings.save() {
            eprintln!("warning: could not save the refreshed account: {e:#}");
        }
    }
}

fn cmd_import(files: &[String]) -> Result<()> {
    if files.is_empty() {
        bail!("no files given");
    }
    let settings = Settings::load();
    let reporter = stdout_reporter();
    for file in files {
        let imported = import::import(&settings, Path::new(file), &reporter)?;
        let inst = &imported.instance;
        println!("Imported \"{}\" ({})", inst.name, inst.description());
        if !imported.blocked.is_empty() {
            println!(
                "{} file(s) must be downloaded by hand, see {}",
                imported.blocked.len(),
                inst.dir.join(import::BLOCKED_LIST).display()
            );
        }
    }
    Ok(())
}

fn cmd_versions(args: &[String]) -> Result<()> {
    let all = args.iter().any(|a| a == "--all");
    let settings = Settings::load();
    for v in version::list(&settings.data_dir)? {
        if all || v.kind == "release" || v.kind == "local" {
            let mark = if v.installed { "*" } else { " " };
            println!("{mark} {:<24} {}", v.id, v.kind);
        }
    }
    Ok(())
}

fn cmd_instances() -> Result<()> {
    let settings = Settings::load();
    for inst in instance::list(&settings.data_dir) {
        println!("{:<32} {}", inst.name, inst.description());
    }
    Ok(())
}
