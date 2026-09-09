//! `pitwall` CLI entry point.
//!
//! M0: prints the version and exits 0. Subcommands
//! (`status`, `checkpoint`, `resume`) arrive in M1/M4.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("--version") | Some("-V") => {
            println!("pitwall {}", pitwall_lib::version());
            ExitCode::SUCCESS
        }
        Some("--help") | Some("-h") => {
            println!(
                "pitwall {} — workspace awareness for Omarchy",
                pitwall_lib::version()
            );
            println!();
            println!("USAGE:");
            println!("    pitwall [OPTIONS]");
            println!();
            println!("OPTIONS:");
            println!("    -V, --version    Print version and exit");
            println!("    -h, --help       Print this help and exit");
            println!();
            println!("NOTE:");
            println!("    Full subcommands arrive in M1+. See ROADMAP.md.");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("pitwall: unknown option '{other}'. Run `pitwall --help`.");
            ExitCode::from(2)
        }
    }
}
