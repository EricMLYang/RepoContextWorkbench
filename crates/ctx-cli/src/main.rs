//! bin `ctx`。

fn main() {
    std::process::exit(ctx_cli::cli::run(std::env::args_os()));
}
