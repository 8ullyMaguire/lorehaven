//! Lorehaven server binary.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    lorehaven_app::main_entry().await
}
