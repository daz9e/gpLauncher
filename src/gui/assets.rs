//! Icons compiled into the binary (Lucide, ISC license; brand logos from Simple Icons, CC0).

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../../assets/icons/", $name, ".svg")))),*]
    };
}

const ICONS: &[(&str, &[u8])] = icons![
    "box",
    "check",
    "chevron-down",
    "chevron-right",
    "coffee",
    "copy",
    "folder",
    "globe",
    "modrinth",
    "monitor",
    "pencil",
    "play",
    "plus",
    "search",
    "settings",
    "share",
    "shortcut",
    "stop",
    "tag",
    "trash",
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS.iter().find(|(p, _)| *p == path).map(|(_, data)| Cow::Borrowed(*data)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| (*p).into()).collect())
    }
}
