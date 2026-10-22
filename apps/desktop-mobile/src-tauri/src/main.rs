// Desktop entry point; mobile starts at `shell::run` (see `tauri::mobile_entry_point`).
fn main() {
    desktop_mobile::shell::run();
}
