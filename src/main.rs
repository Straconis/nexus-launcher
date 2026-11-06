//! Nexus Launcher binary. `--check` parses the catalog and skips the window.

#![forbid(unsafe_code)]

fn main() {
    let code =
        nexus_launcher::cli::execute(nexus_launcher::cli::parse_args(std::env::args().skip(1)));
    if code != 0 {
        std::process::exit(code);
    }
}
