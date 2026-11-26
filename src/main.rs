//! `patch` command-line entry point.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match patch::cli::parse(&args) {
        Ok(options) => {
            let status = patch::run::run(&options);
            std::process::exit(status);
        }
        Err(stop) => patch::cli::report(stop),
    }
}
