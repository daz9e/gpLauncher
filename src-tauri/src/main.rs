//! gpLauncher desktop app.

// Release builds on Windows shouldn't open a console window next to the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    gplauncher_app::run();
}
