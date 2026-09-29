//! Graphical front end for the launcher core.

// Release builds on Windows shouldn't open a console window next to the GUI.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod accounts;
mod add_instance;
mod assets;
mod dropdown;
mod edit_instance;
mod java_field;
mod launcher;
mod modpack_browser;
mod settings_page;
mod shortcut;
mod text_input;
mod theme;

use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions, actions, px, size,
};

use crate::assets::Assets;
use crate::launcher::{FocusSearch, Launcher, NewInstance, OpenSettings};

actions!(gplauncher, [Quit]);

fn main() {
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-n", NewInstance, None),
            KeyBinding::new("cmd-f", FocusSearch, None),
            KeyBinding::new("cmd-,", OpenSettings, None),
        ]);
        text_input::bind_keys(cx);
        accounts::bind_keys(cx);
        add_instance::bind_keys(cx);
        edit_instance::bind_keys(cx);
        settings_page::bind_keys(cx);
        cx.set_menus(vec![
            Menu {
                name: "gpLauncher".into(),
                items: vec![
                    MenuItem::action("Settings…", OpenSettings),
                    MenuItem::separator(),
                    MenuItem::action("Quit", Quit),
                ],
            },
            Menu { name: "File".into(), items: vec![MenuItem::action("Add Instance…", NewInstance)] },
        ]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1080.), px(700.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // The native title bar keeps window controls right on every platform.
                titlebar: Some(TitlebarOptions { title: Some("gpLauncher".into()), ..Default::default() }),
                window_min_size: Some(size(px(680.), px(440.))),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let mut launcher = Launcher::new(window, cx);
                    if let Some(id) = shortcut::launch_arg() {
                        launcher.launch_id(&id, window, cx);
                    }
                    launcher
                })
            },
        )
        .expect("failed to open the main window");
        cx.activate(true);
    });
}
