//! `patch` command-line entry point.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match patch::cli::parse(&args) {
        Ok(_options) => {
            // Application is filled in by later commits. Exit 2 so a
            // successful parse is not mistaken for a successful patch.
            std::process::exit(patch::cli::EXIT_TROUBLE);
        }
        Err(stop) => patch::cli::report(stop),
    }
}
