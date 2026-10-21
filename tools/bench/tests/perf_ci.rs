//! `.github/scripts/perf-ci-v0.mjs` with the built ep9-bench (W4-PERF-REFUSE): neither the check
//! run nor the first counted run vouches for the later counted runs. The engine
//! (`testdata/right-then-wrong.sh`, `RIGHT_RUNS=2`) prints the right answer in the check run and in
//! counted run 1, and a wrong one in counted run 2. The workload must be refused with no count, and
//! the script must exit 1.
//!
//! Needs node (`EP9_NODE` names it); without node the test prints SKIPPED and passes. Unix only:
//! the engine is a shell script.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_ep9-bench");

#[test]
fn a_wrong_answer_in_a_counted_run_is_refused() {
    let node = std::env::var("EP9_NODE").unwrap_or_else(|_| "node".into());
    if Command::new(&node).arg("--version").output().is_err() {
        eprintln!(
            "a_wrong_answer_in_a_counted_run_is_refused: SKIPPED, node is not installed (set \
             EP9_NODE to its path)"
        );
        return;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!("ep9-perf-ci-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // `auto`: this host's counter, if it has one. `cachegrind`: the counter of the CI job.
    for counter in ["auto", "cachegrind"] {
        let state = dir.join(format!("{counter}.runs"));
        let json = dir.join(format!("{counter}.json"));
        // The engine's run count starts at 0.
        let _ = std::fs::remove_file(&state);
        let out = Command::new(&node)
            .arg(manifest.join("../../.github/scripts/perf-ci-v0.mjs"))
            .arg("--ep9")
            .arg(manifest.join("tests/testdata/right-then-wrong.sh"))
            .args([
                "--bench",
                BIN,
                "--counter",
                counter,
                "--runs",
                "3",
                "--warmup",
                "0",
            ])
            .args(["--workloads", "t0-call", "--out"])
            .arg(&json)
            .env("RIGHT_RUNS", "2")
            .env("RIGHT_STATE", &state)
            .output()
            .expect("run node");
        let stderr = String::from_utf8_lossy(&out.stderr);
        let report = std::fs::read_to_string(&json).unwrap_or_else(|e| panic!("{e}: {stderr}"));
        assert_eq!(out.status.code(), Some(1), "{counter}: {stderr}\n{report}");
        // Three runs: the check run and counted run 1 passed, then ep9-bench refused counted run 2
        // and made no counted run 3.
        let runs = std::fs::read_to_string(&state).unwrap();
        assert_eq!(runs.trim(), "3", "{counter}: {report}");
        for want in [
            r#""complete": false"#,
            r#""status": "refused""#,
            r#""instructions": null"#,
            r#""bench": null"#,
            r#"printed \"12345\", want \"577789\""#,
        ] {
            assert!(report.contains(want), "{counter}: no {want}: {report}");
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
