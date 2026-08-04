use std::process::ExitCode;

use fkst_local_qa_host::{validate_startup, StartupInput};

fn main() -> ExitCode {
    match validate_startup(StartupInput::default()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fkst-local-qa-host: {error}");
            ExitCode::FAILURE
        }
    }
}
