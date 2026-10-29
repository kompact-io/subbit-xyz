//! Thin binary entry point -- all the actual argument parsing and
//! dispatch lives in `subbit_console::ui::cli` (part of the library,
//! not this binary), so a future native TUI can reuse that adapter
//! shape without duplicating it. See `ui::cli`'s own doc.

fn main() {
    if let Err(e) = subbit_console::ui::cli::run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
