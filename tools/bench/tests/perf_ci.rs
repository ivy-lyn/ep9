//! `.github/scripts/perf-ci-v0.mjs` with the built ep9-bench (W4-PERF-REFUSE): the check run does
//! not vouch for the counted runs. The engine (`testdata/right-once.sh`) prints the right answer in
//! the check run and a wrong one in every later run. The workload must be refused with no count,
//! and the script must exit 1.
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
        let out = Command::new(&node)
            .arg(manifest.join("../../.github/scripts/perf-ci-v0.mjs"))
            .arg("--ep9")
            .arg(manifest.join("tests/testdata/right-once.sh"))
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
            .env("RIGHT_ONCE_STATE", &state)
            .output()
            .expect("run node");
        let stderr = String::from_utf8_lossy(&out.stderr);
        let report = std::fs::read_to_string(&json).unwrap_or_else(|e| panic!("{e}: {stderr}"));
        assert_eq!(out.status.code(), Some(1), "{counter}: {stderr}\n{report}");
        // Two runs: the check run passed, then ep9-bench refused its first run.
        let runs = std::fs::read_to_string(&state).unwrap();
        assert_eq!(runs.trim(), "2", "{counter}: {report}");
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
