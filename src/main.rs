fn main() {
    if let Err(err) = bear_cli::run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
