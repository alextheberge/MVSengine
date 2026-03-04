// SPDX-License-Identifier: AGPL-3.0-only
fn main() {
    if let Err(error) = mvs_manager::cli::run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
