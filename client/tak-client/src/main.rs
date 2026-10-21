use clap::Parser;

fn main() -> std::process::ExitCode {
    let args = tak_client::cli::Args::parse();
    match tak_client::app::run(args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tak-client: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
