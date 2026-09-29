//! Graphical front end for the launcher core.

mod add_instance;
mod assets;
mod dropdown;
mod edit_instance;
mod launcher;
mod modpack_browser;
mod shortcut;
mod text_input;
mod theme;

use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions, actions, px, size,
};

use crate::assets::Assets;
use crate::launcher::{FocusSearch, Launcher, NewInstance};

actions!(gplauncher, [Quit]);

fn main() {
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-n", NewInstance, None),
            KeyBinding::new("cmd-f", FocusSearch, None),
        ]);
        text_input::bind_keys(cx);
        add_instance::bind_keys(cx);
        edit_instance::bind_keys(cx);
        cx.set_menus(vec![
            Menu { name: "gpLauncher".into(), items: vec![MenuItem::action("Quit", Quit)] },
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
                        launcher.launch_id(&id, cx);
                    }
                    launcher
                })
            },
        )
        .expect("failed to open the main window");
        cx.activate(true);
    });
}
