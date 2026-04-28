use std::process::ExitCode;

use clipt9n::run;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("clipt9n: {e}");
            ExitCode::FAILURE
        }
    }
}
