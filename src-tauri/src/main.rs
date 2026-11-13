// No console window behind the app on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // On Windows, Go Max stops the node by starting a copy of this program
    // as a short-lived helper. That copy does its one job and exits here.
    if let Some(code) = lockit_node::process::run_stop_helper_if_asked() {
        std::process::exit(code);
    }
    zecky_lock_it::run()
}
