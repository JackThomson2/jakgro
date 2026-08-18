fn main() -> std::process::ExitCode {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let output = std::io::BufWriter::new(stdout.lock());

    match jakgro::uci::run(stdin.lock(), output) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jakgro: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
