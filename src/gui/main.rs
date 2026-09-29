//! Graphical front end for the launcher core.

mod assets;
mod launcher;
mod theme;

use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions, actions, point, px, size,
};

use crate::assets::Assets;
use crate::launcher::{Launcher, TOOLBAR_HEIGHT};

actions!(gplauncher, [Quit]);

fn main() {
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu { name: "gpLauncher".into(), items: vec![MenuItem::action("Quit", Quit)] }]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(960.), px(620.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("gpLauncher".into()),
                    appears_transparent: true,
                    // Centered vertically in the toolbar (the buttons are ~14px tall).
                    traffic_light_position: Some(point(px(18.), px((TOOLBAR_HEIGHT - 14.) / 2.))),
                }),
                window_min_size: Some(size(px(680.), px(440.))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Launcher::new(window, cx)),
        )
        .expect("failed to open the main window");
        cx.activate(true);
    });
}
