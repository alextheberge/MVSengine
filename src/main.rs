fn main() {
    if let Err(error) = mvs_manager::cli::run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
