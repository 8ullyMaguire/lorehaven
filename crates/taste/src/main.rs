//! The `taste` binary.
//!
//! **It does nothing yet, and it says so.** This is the service mode of the taste engine
//! (spec §8.2): a separate process against the same catalogue bundle, for an instance
//! whose graph does not fit beside the archive's request/response process.
//!
//! The discipline this binary follows from the start: **a stub that returns a plausible
//! number is worse than no stub at all**, because a stub is a ranking that looks measured
//! and is not. So the binary prints what it is and exits 0. The catalogue ingest, the
//! `/healthz` that returns 503 over the RSS cap, and the budget gate come with the
//! modules that can honour them.

fn main() {
    println!("darkhold-taste — the taste engine (spec §8.2)");
    println!("catalogue ingest, the ranker and the budget gate are not built yet.");
    std::process::exit(0);
}
