//! Ask the configured decision model one yes/no question and print the answer.
//!
//! Exists because "the model is deployed" and "the model answers" are
//! different claims, and only this one tests the second from the host that will
//! actually run it. A curl in a shell proves the server works; it does not
//! prove Lorehaven's client can reach it, parse what it says, and refuse an
//! answer it should refuse.
//!
//! ```console
//! $ cargo run -p lorehaven-decisions --bin probe
//! $ LOREHAVEN_DECISIONS_API_KEY=sk-unsloth-... cargo run -p lorehaven-decisions --bin probe
//! ```

use lorehaven_decisions::{Client, ClientConfig, Question};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ClientConfig {
        base_url: std::env::var("LOREHAVEN_DECISIONS_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8888".to_owned()),
        model: std::env::var("LOREHAVEN_DECISIONS_MODEL").unwrap_or_else(|_| "laya".to_owned()),
        api_key: std::env::var("LOREHAVEN_DECISIONS_API_KEY").ok(),
        timeout_ms: 30_000,
    };
    let state = std::env::args().nth(1).unwrap_or_else(|| {
        "Hi, I was charged twice for my March invoice. Please refund it.".to_owned()
    });

    let client = Client::new(config);
    println!("asking {} for one question", client.base_url());

    let questions = vec![Question::YesNo {
        key: "refund".to_owned(),
        instructions: "Does the customer ask for a refund?".to_owned(),
    }];

    match client.ask(&serde_json::json!(state), &questions).await {
        Ok(decisions) => {
            for decision in decisions {
                let probability = decision.answer.probability_of_yes();
                println!(
                    "{} = {}",
                    decision.key,
                    probability
                        .map(|p| format!("{p:.4}"))
                        .unwrap_or_else(|| "<not a yes/no question>".to_owned())
                );
            }
            Ok(())
        }
        // A refusal is reported as a refusal and exits non-zero. This probe
        // exists to be honest, and a probe that prints "fine" when the model
        // is down is the same bug as a decision path that silently degrades.
        Err(error) => {
            println!("the model did not answer: {error}");
            if error.is_transient() {
                println!("(transient — the instance would fall back to the deterministic path)");
            }
            // `DecisionError` is not `std::error::Error + Send + Sync +
            // 'static` through this path, so the message is the exit. A probe
            // that exits zero after printing a refusal is a probe nobody reads.
            std::process::exit(1);
        }
    }
}
