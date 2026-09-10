//! Darkhold server binary.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    darkhold_app::main_entry().await
}
