#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> std::process::ExitCode {
    let input = std::io::BufReader::new(std::io::stdin());
    let stdout = std::io::stdout();
    let output = std::io::BufWriter::new(stdout.lock());

    match jakgro::uci::run(input, output) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jakgro: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
