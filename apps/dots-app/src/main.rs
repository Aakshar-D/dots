// Release builds are GUI apps: no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    dots_app::run()
}
