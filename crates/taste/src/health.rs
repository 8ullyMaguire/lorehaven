//! The engine's own health.
//!
//! **A stub `200` is worse than nothing**, because it hides the failure the cap
//! exists to catch: a `/healthz` that always answers `200` is a green light on
//! a process that is about to be killed by the OOM killer, and the operator
//! finds out from the outage rather than from the health check.
//!
//! So this module reads the process's **actual** resident set and compares it
//! with the cap. There is no configuration that makes it answer `200` while the
//! process is over its cap.
//!
//! The measurement is the process's own, from `/proc/self/status`. That is the
//! only number that describes *this* process: a supervisor's view of the
//! service is the supervisor's, and the service's cap is the service's.

/// The full engine's cap: **220 MiB** (spec §22.3, from kindred's measured
/// `embed` peak of 203.2 MB).
pub const CAP_FULL_BYTES: u64 = 220 * 1024 * 1024;

/// The lite engine's cap: **60 MiB** (spec §22.3, from kindred's measured
/// `recommend` peak of 25–26 MiB).
pub const CAP_LITE_BYTES: u64 = 60 * 1024 * 1024;

/// The process's resident set size, in bytes.
///
/// `None` when the platform does not say — **and `None` is not "under the
/// cap".** A platform that cannot answer is a platform whose state is unknown,
/// and an unknown state is not a pass.
#[must_use]
pub fn rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        let Some(rest) = line.strip_prefix("VmRSS:") else {
            continue;
        };
        let kib: u64 = rest.split_whitespace().next()?.parse().ok()?;
        return Some(kib * 1024);
    }
    None
}

/// The peak resident set size, in bytes — the kernel's own high-water mark.
///
/// `VmHWM` is the peak, and a cap is about the peak: a process that allocated
/// 300 MiB and released it is not over its cap now, but it *was*, and the
/// number that decides whether the next allocation is safe is the peak.
#[must_use]
pub fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        let Some(rest) = line.strip_prefix("VmHWM:") else {
            continue;
        };
        let kib: u64 = rest.split_whitespace().next()?.parse().ok()?;
        return Some(kib * 1024);
    }
    None
}

/// The engine's health, and the reason for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    /// Whether the process is within its cap.
    pub ok: bool,
    /// The cap the process was measured against.
    pub cap_bytes: u64,
    /// The process's resident set, when the platform says.
    pub rss_bytes: Option<u64>,
    /// The process's peak resident set, when the platform says.
    pub peak_rss_bytes: Option<u64>,
    /// Why the answer is what it is.
    pub detail: String,
}

/// Check the process against `cap_bytes`.
///
/// **An unreadable measurement is a failure, not a pass.** A process that cannot
/// say how big it is cannot say it is within its cap, and the alternative — a
/// silent `200` — is exactly the stub this module exists to refuse.
#[must_use]
pub fn check(cap_bytes: u64) -> Health {
    let rss = rss_bytes();
    let peak = peak_rss_bytes();

    let (ok, detail) = match (rss, peak) {
        (Some(rss), Some(peak)) => {
            let used = rss.max(peak);
            if used > cap_bytes {
                (
                    false,
                    format!("{used} bytes resident (peak {peak}), over the {cap_bytes}-byte cap"),
                )
            } else {
                (
                    true,
                    format!("{used} bytes resident (peak {peak}), within the {cap_bytes}-byte cap"),
                )
            }
        }
        (Some(rss), None) => {
            if rss > cap_bytes {
                (
                    false,
                    format!("{rss} bytes resident, over the {cap_bytes}-byte cap"),
                )
            } else {
                (
                    true,
                    format!("{rss} bytes resident, within the {cap_bytes}-byte cap"),
                )
            }
        }
        _ => (
            false,
            "the process cannot say how much memory it is using, so it cannot say it is \
             within its cap"
                .to_string(),
        ),
    };

    Health {
        ok,
        cap_bytes,
        rss_bytes: rss,
        peak_rss_bytes: peak,
        detail,
    }
}
