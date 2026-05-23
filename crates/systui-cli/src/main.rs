//! SysTUI command-line entry point.
//!
//! The real CLI (argument parsing, config loading, mode selection, TUI launch)
//! is built in phase 0, session S0.5. This is the scaffold entry point.

fn main() {
    println!("systui {} (scaffold)", env!("CARGO_PKG_VERSION"));
}
