use std::process::ExitCode;

use fkst_local_qa_host::{parse_startup, run};

fn main() -> ExitCode {
    let config = match parse_startup(std::env::args_os().skip(1)) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("fkst-local-qa-host: {error}");
            return ExitCode::FAILURE;
        }
    };

    match run(config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fkst-local-qa-host: {error}");
            ExitCode::FAILURE
        }
    }
}
