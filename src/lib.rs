//! Minecraft: Java Edition launcher core.
//! All functions here are blocking and are meant to run on worker threads;
//! progress is reported through a [`Reporter`].

pub mod addons;
pub mod auth;
pub mod content;
pub mod curseforge;
pub mod download;
pub mod export;
pub mod forge;
pub mod http;
pub mod import;
pub mod install;
pub mod instance;
pub mod java;
pub mod launch;
pub mod loader;
pub mod modpack;
pub mod modrinth;
pub mod nbt;
pub mod platform;
pub mod settings;
pub mod version;

use std::sync::Arc;
use std::sync::mpsc::Sender;

use auth::Account;
use instance::Instance;

#[derive(Debug)]
pub enum Event {
    Status(String),
    Progress {
        done: u64,
        total: u64,
    },
    Log(String),
    /// A Microsoft account was refreshed; the caller should persist it.
    AccountRefreshed(Account),
    /// An instance changed on disk (e.g. its loader version got resolved).
    InstanceUpdated(Instance),
    GameStarted,
    GameExited(Option<i32>),
}

#[derive(Clone)]
pub struct Reporter(Arc<dyn Fn(Event) + Send + Sync>);

impl Reporter {
    pub fn new(f: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    /// Reporter that drops every event.
    pub fn silent() -> Self {
        Self::new(|_| {})
    }

    pub fn send(&self, event: Event) {
        (self.0)(event);
    }

    pub fn status(&self, s: impl Into<String>) {
        self.send(Event::Status(s.into()));
    }

    pub fn log(&self, s: impl Into<String>) {
        self.send(Event::Log(s.into()));
    }

    pub fn progress(&self, done: u64, total: u64) {
        self.send(Event::Progress { done, total });
    }
}

impl From<Sender<Event>> for Reporter {
    fn from(tx: Sender<Event>) -> Self {
        Self::new(move |event| {
            let _ = tx.send(event);
        })
    }
}
