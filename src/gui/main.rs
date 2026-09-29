//! Graphical front end for the launcher core.

// Release builds on Windows shouldn't open a console window next to the GUI.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod accounts;
mod add_instance;
mod addon_browser;
mod assets;
mod content_page;
mod dropdown;
mod instance_settings;
mod instance_window;
mod java_field;
mod launcher;
mod log_view;
mod modpack_browser;
mod pages;
mod settings_page;
mod shortcut;
mod state;
mod text_input;
mod theme;
mod ui;

use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions, actions, px, size,
};

use crate::assets::Assets;
use crate::launcher::{FocusSearch, LaunchSelected, Launcher, NewInstance, OpenSelected, OpenSettings};
use crate::state::AppState;

actions!(gplauncher, [Quit]);

fn main() {
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-n", NewInstance, None),
            KeyBinding::new("cmd-f", FocusSearch, None),
            KeyBinding::new("cmd-,", OpenSettings, None),
            KeyBinding::new("cmd-enter", LaunchSelected, None),
            KeyBinding::new("cmd-o", OpenSelected, None),
        ]);
        text_input::bind_keys(cx);
        accounts::bind_keys(cx);
        add_instance::bind_keys(cx);
        instance_window::bind_keys(cx);
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
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("Add Instance…", NewInstance),
                    MenuItem::separator(),
                    MenuItem::action("Play Selected", LaunchSelected),
                    MenuItem::action("Open Selected", OpenSelected),
                ],
            },
        ]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let state = AppState::new(cx);
        #[cfg(debug_assertions)]
        let state_for_debug = state.clone();
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
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
                    let mut launcher = Launcher::new(state, window, cx);
                    if let Some(id) = shortcut::launch_arg() {
                        launcher.launch_id(&id, cx);
                    }
                    launcher
                })
            },
        )
        .expect("failed to open the main window");
        // Development aid: `GPLAUNCHER_OPEN=<instance id>:<page>` opens an instance window.
        #[cfg(debug_assertions)]
        if let Ok(spec) = std::env::var("GPLAUNCHER_OPEN") {
            instance_window::open_debug(&state_for_debug, &spec, cx);
        }
        cx.activate(true);
    });
}
