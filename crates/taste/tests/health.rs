//! The health check is real, and it is tested as one.
//!
//! **A stub `200` is worse than nothing**, because it hides the failure the cap
//! exists to catch. So the test drives the cap to a number the process cannot
//! possibly be under and asserts the answer is a refusal — and drives it to a
//! number the process is certainly under and asserts the answer is an
//! acceptance. **A test that only ever sees the passing case proves that the
//! function returns `true`.**

use darkhold_taste::health;

#[test]
fn the_process_can_say_how_much_memory_it_is_using() {
    // On Linux this reads `/proc/self/status`. The test is skipped elsewhere
    // rather than failed, because the *platform* is the thing that varies and
    // the contract is "the platform cannot answer is not a pass".
    match health::rss_bytes() {
        Some(bytes) => {
            assert!(bytes > 0, "a process with no memory is not a process");
        }
        None => {
            // Not Linux, or no `/proc`. The contract still holds: an unknown
            // measurement is a failure.
            let report = health::check(health::CAP_FULL_BYTES);
            assert!(
                !report.ok,
                "an unreadable measurement must not be reported as within the cap"
            );
        }
    }
}

#[test]
fn a_process_over_its_cap_says_so() {
    // A cap of one byte cannot be met by a running process. The check has to
    // refuse, and the detail has to name the numbers — a refusal that does not
    // say how far over it is costs the operator the run to find out.
    let report = health::check(1);
    assert!(!report.ok, "a one-byte cap must be refused");
    assert!(
        report.detail.contains("over"),
        "the detail says it is over: {}",
        report.detail
    );
    assert_eq!(report.cap_bytes, 1);
}

#[test]
fn a_process_within_its_cap_says_so() {
    // A cap of a terabyte cannot be exceeded by a test process. The check has
    // to accept, and the detail has to say it is within.
    let report = health::check(1024 * 1024 * 1024 * 1024);
    assert!(
        report.ok,
        "a terabyte cap must be accepted: {}",
        report.detail
    );
    assert!(
        report.detail.contains("within"),
        "the detail says it is within: {}",
        report.detail
    );
}

#[test]
fn the_caps_are_the_specifications_numbers() {
    // 220 MiB for the full engine and 60 MiB for the lite one (spec §22.3).
    assert_eq!(health::CAP_FULL_BYTES, 220 * 1024 * 1024);
    assert_eq!(health::CAP_LITE_BYTES, 60 * 1024 * 1024);
    // A constant assertion, so it is a constant assertion: the compiler checks
    // it once, at compile time, and a change to either cap that breaks the
    // ordering does not build.
    const { assert!(health::CAP_LITE_BYTES < health::CAP_FULL_BYTES) };
}

#[test]
fn the_peak_is_at_least_the_current() {
    // `VmHWM` is the kernel's high-water mark, so it cannot be below the
    // current resident set. If it ever is, the two are being read from
    // different processes and the measurement is meaningless.
    let (Some(rss), Some(peak)) = (health::rss_bytes(), health::peak_rss_bytes()) else {
        return; // the platform does not say
    };
    assert!(
        peak >= rss,
        "the peak ({peak}) cannot be below the current resident set ({rss})"
    );
}
