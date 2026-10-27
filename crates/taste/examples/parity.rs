//! The parity harness: turn "we match kindred" into a number.
//!
//! ```text
//! cargo run -p darkhold-taste --example parity -- \
//!     --judged judged.json --oracle oracle.json --candidate candidate.json
//! ```
//!
//! Three inputs, because they come from three different places and merging them by hand is
//! how an evaluation quietly becomes fiction:
//!
//! ```text
//! judged.json     { "readers": [ { "reader": "r1", "judged": [ {"work_id": "w1", "grade": 2} ] } ] }
//! oracle.json     { "readers": [ { "reader": "r1", "ranking": ["w1", "w9"] } ] }
//! candidate.json  { "readers": [ { "reader": "r1", "ranking": ["w1", "w7"] } ] }
//! ```
//!
//! `oracle` is kindred's recommendation for that reader over its own AO3 mirror; `candidate`
//! is Darkhold's blend for the same reader and the same works. The harness does not care
//! which engine produced which file — it scores both against the same judgements, so the
//! comparison survives either side being replaced.
//!
//! # What it refuses to do
//!
//! - **It does not invent a missing reader.** A reader present in the judgements but absent
//!   from a ranking is counted and named. Silently scoring them as an empty ranking would
//!   report a loss the engine did not take; silently skipping them would report a win.
//! - **It does not average over unscored readers.** See [`darkhold_taste::eval`].
//! - **It does not exit 0 on a run with nothing to compare.** A harness that prints "n/a" and
//!   succeeds is how a parity claim gets made from an empty file.

use darkhold_taste::eval::{Judgement, ParityReport, ReaderCase, ReaderScore};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::process::ExitCode;

#[derive(Debug, Deserialize)]
struct JudgedFile {
    readers: Vec<JudgedReader>,
}

#[derive(Debug, Deserialize)]
struct JudgedReader {
    reader: String,
    #[serde(default)]
    judged: Vec<JudgementJson>,
}

#[derive(Debug, Deserialize)]
struct JudgementJson {
    work_id: String,
    #[serde(default = "one")]
    grade: u32,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Deserialize)]
struct RankingFile {
    readers: Vec<RankingReader>,
}

#[derive(Debug, Deserialize)]
struct RankingReader {
    reader: String,
    #[serde(default)]
    ranking: Vec<String>,
}

fn read<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    serde_json::from_str(&text).map_err(|error| format!("{path}: {error}"))
}

fn main() -> ExitCode {
    let mut judged_path = None;
    let mut oracle_path = None;
    let mut candidate_path = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--judged" => judged_path = args.next(),
            "--oracle" => oracle_path = args.next(),
            "--candidate" => candidate_path = args.next(),
            "--help" | "-h" => {
                println!("usage: parity --judged J --oracle O --candidate C");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let (Some(judged_path), Some(oracle_path), Some(candidate_path)) =
        (judged_path, oracle_path, candidate_path)
    else {
        eprintln!("usage: parity --judged J --oracle O --candidate C");
        return ExitCode::from(2);
    };

    let judged: JudgedFile = match read(&judged_path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    let oracle: RankingFile = match read(&oracle_path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    let candidate: RankingFile = match read(&candidate_path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };

    // BTreeMap so the run is deterministic: two harness runs over the same files must print
    // the same report, and a HashMap would reorder the per-reader lines between runs.
    let mut oracle_rankings: BTreeMap<String, Vec<String>> = oracle
        .readers
        .into_iter()
        .map(|reader| (reader.reader, reader.ranking))
        .collect();
    let mut candidate_rankings: BTreeMap<String, Vec<String>> = candidate
        .readers
        .into_iter()
        .map(|reader| (reader.reader, reader.ranking))
        .collect();

    let mut cases = Vec::new();
    let mut missing_oracle = Vec::new();
    let mut missing_candidate = Vec::new();

    for reader in judged.readers {
        let judged: Vec<Judgement> = reader
            .judged
            .into_iter()
            .map(|judgement| Judgement {
                work_id: judgement.work_id,
                grade: judgement.grade,
            })
            .collect();

        let oracle = oracle_rankings.remove(&reader.reader);
        let candidate = candidate_rankings.remove(&reader.reader);

        match (oracle, candidate) {
            (Some(oracle), Some(candidate)) => cases.push(ReaderCase {
                reader: reader.reader,
                judged,
                candidate,
                oracle,
            }),
            (oracle, candidate) => {
                if oracle.is_none() {
                    missing_oracle.push(reader.reader.clone());
                }
                if candidate.is_none() {
                    missing_candidate.push(reader.reader.clone());
                }
            }
        }
    }

    let report = ParityReport::build(&cases);
    for line in report.lines() {
        println!("{line}");
    }

    if !missing_oracle.is_empty() {
        println!(
            "\n{} reader(s) judged but not ranked by the oracle: {}",
            missing_oracle.len(),
            missing_oracle.join(", ")
        );
    }
    if !missing_candidate.is_empty() {
        println!(
            "\n{} reader(s) judged but not ranked by the candidate: {}",
            missing_candidate.len(),
            missing_candidate.join(", ")
        );
    }

    // The readers where the candidate loses, worst first. A mean is the headline; this is the
    // part someone can act on.
    let mut losses: Vec<(f64, &ReaderCase, ReaderScore)> = cases
        .iter()
        .filter_map(|case| {
            let score = ReaderScore::of(case);
            let oracle = darkhold_taste::eval::ndcg_at(&case.oracle, &case.judged, 10)?;
            let candidate = score.ndcg_at_10?;
            Some((candidate - oracle, case, score))
        })
        .filter(|(delta, _, _)| *delta < -1e-9)
        .collect();
    losses.sort_by(|a, b| a.0.total_cmp(&b.0));

    if !losses.is_empty() {
        println!("\nreaders where the candidate trails the oracle (nDCG@10):");
        for (delta, case, score) in &losses {
            println!(
                "  {:+0.4}  {}  (candidate {}, overlap@10 {:0.2})",
                delta,
                case.reader,
                score
                    .ndcg_at_10
                    .map_or_else(|| "n/a".to_string(), |v| format!("{v:.4}")),
                score.overlap_at_10
            );
        }
    }

    if report.scored == 0 {
        eprintln!(
            "\nno reader could be scored: {} judged, {} ranked on both sides. \
             This run proves nothing and is not a pass.",
            report.readers,
            report.scored
        );
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
