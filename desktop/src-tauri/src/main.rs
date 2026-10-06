// release 版在 Windows 不要多開一個主控台視窗
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    repo_context_desktop_lib::run()
}
