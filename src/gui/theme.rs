//! Neutral palette; the accent is reserved for selection and the primary action.

use gpui::{Hsla, WindowAppearance, rgb};

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Hsla,
    pub panel: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub subtle: Hsla,
    pub hover: Hsla,
    pub tile: Hsla,
    pub tile_edge: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub on_accent: Hsla,
    pub danger: Hsla,
}

impl Theme {
    pub fn for_appearance(appearance: WindowAppearance) -> Theme {
        let (accent, dark) = match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => (rgb(0x4c8dff).into(), true),
            WindowAppearance::Light | WindowAppearance::VibrantLight => (rgb(0x2f6bef).into(), false),
        };
        let accent: Hsla = accent;
        if dark {
            Theme {
                bg: rgb(0x1b1b1d).into(),
                panel: rgb(0x212124).into(),
                border: rgb(0x2e2e32).into(),
                text: rgb(0xececef).into(),
                muted: rgb(0x9a9aa2).into(),
                subtle: rgb(0x6c6c74).into(),
                hover: rgb(0x2a2a2e).into(),
                tile: rgb(0x2c2c31).into(),
                tile_edge: rgb(0x38383e).into(),
                accent,
                accent_soft: accent.opacity(0.16),
                on_accent: rgb(0xffffff).into(),
                danger: rgb(0xff6b6b).into(),
            }
        } else {
            Theme {
                bg: rgb(0xffffff).into(),
                panel: rgb(0xf7f7f8).into(),
                border: rgb(0xe7e7ea).into(),
                text: rgb(0x1c1c1f).into(),
                muted: rgb(0x6e6e76).into(),
                subtle: rgb(0xa1a1a8).into(),
                hover: rgb(0xf0f0f2).into(),
                tile: rgb(0xf1f1f3).into(),
                tile_edge: rgb(0xe3e3e7).into(),
                accent,
                accent_soft: accent.opacity(0.09),
                on_accent: rgb(0xffffff).into(),
                danger: rgb(0xd92d20).into(),
            }
        }
    }
}
