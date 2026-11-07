//! Subbit console via default native UI.
//!
fn main() {
    if let Err(e) = subbit_console::ui::cli::run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
