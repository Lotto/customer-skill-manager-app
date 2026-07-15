// Hide the console window on Windows release builds — this is a silent tray app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    customer_skill_manager_lib::run();
}
