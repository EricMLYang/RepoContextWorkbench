//! bin `ctx`。

fn main() {
    restore_sigpipe();
    std::process::exit(ctx_cli::cli::run(std::env::args_os()));
}

/// Rust 會忽略 SIGPIPE，讀的那端（`ctx … | head`、提早關掉的 hook／agent）一關，`println!` 就 panic
/// 「failed printing to stdout: Broken pipe」。恢復成系統預設＝安靜結束，同一般 Unix 指令（ideation N-27）。
#[cfg(unix)]
fn restore_sigpipe() {
    // SAFETY: 只在 main 一開始、還沒有其他執行緒時設定一次訊號處理。
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_sigpipe() {}
