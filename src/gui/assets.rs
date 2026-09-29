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
    "arrow-down",
    "box",
    "check",
    "chevron-down",
    "chevron-left",
    "chevron-right",
    "circle-arrow-up",
    "clock",
    "coffee",
    "copy",
    "download",
    "earth",
    "ellipsis",
    "external-link",
    "file-text",
    "folder",
    "gamepad-2",
    "globe",
    "hard-drive",
    "image",
    "info",
    "layers",
    "modrinth",
    "monitor",
    "pencil",
    "pickaxe",
    "play",
    "plus",
    "puzzle",
    "refresh-cw",
    "search",
    "settings",
    "share",
    "shortcut",
    "sliders-horizontal",
    "sparkles",
    "stop",
    "tag",
    "terminal",
    "trash",
    "triangle-alert",
    "upload",
    "user-round",
    "x",
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
