//! Runs the built `ep9-bench`. A host either gives instruction counts, or ep9-bench writes the
//! JSON with `"counter_status": "unavailable"` and null counts, and exits 3.
//!
//! The tests do not ask ep9-bench which case applies. `host_counter` runs the OS tool itself and
//! reads its output, so a bug in ep9-bench's own probe cannot turn the count check into a skip.
//! The benchmarked command is `ep9-bench --help`: it exists and exits 0 on every host.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_ep9-bench");

fn bench(args: &[&str], time_tool: Option<&Path>) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.args(args);
    if let Some(t) = time_tool {
        cmd.env("EP9_BENCH_TIME", t);
    }
    cmd.output().expect("run ep9-bench")
}

/// The fake `time -l` of a host with no PMC access (see the script).
#[cfg(unix)]
fn fake_time_no_pmc() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testdata/time-no-pmc.sh")
}

/// The counter id that ep9-bench must pick on this host, or why the host has none. It runs the
/// same OS tools as ep9-bench (the same order on Linux), but parses their output here.
fn host_counter() -> Result<&'static str, String> {
    let tool_output = |program: &str, args: &[&str]| -> Result<String, String> {
        let out = Command::new(program)
            .args(args)
            .output()
            .map_err(|e| format!("cannot run {program}: {e}"))?;
        if !out.status.success() {
            return Err(format!("{program} exited with {}", out.status));
        }
        Ok(String::from_utf8_lossy(&out.stderr).into_owned())
    };
    if cfg!(target_os = "macos") {
        let time = std::env::var("EP9_BENCH_TIME").unwrap_or_else(|_| "/usr/bin/time".into());
        let text = tool_output(&time, &["-l", "/usr/bin/true"])?;
        // Apple's time.c prints this line only for a count above 0.
        // `   10719452  instructions retired`
        let n = text
            .lines()
            .filter_map(|l| l.trim().strip_suffix("instructions retired"))
            .find_map(|n| n.trim().parse::<u64>().ok())
            .unwrap_or(0);
        return if n > 0 {
            Ok("macos-time-l-instructions-retired")
        } else {
            Err(format!(
                "`{time} -l` gives no \"instructions retired\" count"
            ))
        };
    }
    if cfg!(target_os = "linux") {
        let perf = tool_output(
            "perf",
            &["stat", "-x", ",", "-e", "instructions:u", "/bin/true"],
        );
        match perf {
            // CSV `<count>,<unit>,instructions:u,...`; the count is `<not supported>` without access.
            Ok(text)
                if text.lines().any(|l| {
                    let f: Vec<&str> = l.split(',').collect();
                    f.len() > 2
                        && f[2].starts_with("instructions")
                        && f[0].trim().parse::<u64>().is_ok_and(|n| n > 0)
                }) =>
            {
                return Ok("perf-stat-instructions-u");
            }
            _ => {}
        }
        let args = [
            "--tool=cachegrind",
            "--cache-sim=no",
            "--cachegrind-out-file=/dev/null",
            "/bin/true",
        ];
        return match tool_output("valgrind", &args) {
            // `==123== I   refs:      1,234,567`
            Ok(text)
                if text.lines().any(|l| {
                    l.split_once(" refs:").is_some_and(|(_, n)| {
                        n.trim()
                            .replace(',', "")
                            .parse::<u64>()
                            .is_ok_and(|n| n > 0)
                    })
                }) =>
            {
                Ok("cachegrind-i-refs")
            }
            Ok(_) => Err("perf gives no count; valgrind gives no \"I refs\" count".into()),
            Err(e) => Err(format!("perf gives no count; {e}")),
        };
    }
    Err(format!(
        "ep9-bench has no counter for {}",
        std::env::consts::OS
    ))
}

/// Checks the JSON of a host with no counter.
fn assert_no_counter(out: &Output) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let json = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(3), "stderr: {stderr}\njson: {json}");
    assert!(
        stderr.contains("no instruction counter on this host"),
        "{stderr}"
    );
    assert!(json.contains("\"schema\": \"ep9-bench/v1\""), "{json}");
    assert!(json.contains("\"counter\": null"), "{json}");
    assert!(
        json.contains("\"counter_status\": \"unavailable\""),
        "{json}"
    );
    assert!(!json.contains("\"counter_note\": null"), "{json}");
    assert_eq!(
        json.matches("{ \"instructions\": null, \"wall_ns\": ")
            .count(),
        2,
        "{json}"
    );
    assert!(
        json.contains("\"summary\": {\n    \"instructions\": null,"),
        "{json}"
    );
    // A null count is "not measured": no run may claim a number.
    assert!(!json.contains("\"instructions\": 0"), "{json}");
}

#[test]
fn counts_instructions() {
    let out = bench(
        &[
            "--runs", "2", "--warmup", "0", "--name", "tiny", "--", BIN, "--help",
        ],
        None,
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let json = String::from_utf8_lossy(&out.stdout);
    let id = match host_counter() {
        Ok(id) => id,
        Err(why) => {
            // Written past libtest's capture, so the CI log shows the skip.
            let _ = writeln!(
                std::io::stderr(),
                "counts_instructions: SKIPPED the count check, no instruction counter on this \
                 host ({why}). Checked the explicit no-counter JSON instead."
            );
            assert_no_counter(&out);
            return;
        }
    };
    assert_eq!(out.status.code(), Some(0), "stderr: {stderr}\njson: {json}");
    assert!(json.contains("\"schema\": \"ep9-bench/v1\""), "{json}");
    assert!(json.contains(&format!("\"counter\": \"{id}\"")), "{json}");
    assert!(json.contains("\"counter_status\": \"ok\""), "{json}");
    assert!(json.contains("\"counter_note\": null"), "{json}");
    // Every run of a real process retires instructions.
    let counts: Vec<u64> = json
        .lines()
        .filter_map(|l| l.trim().strip_prefix("{ \"instructions\": "))
        .map(|l| {
            let n = &l[..l.find(',').expect("a comma after the count")];
            n.parse()
                .unwrap_or_else(|_| panic!("a number, not {n:?}: {json}"))
        })
        .collect();
    assert_eq!(counts.len(), 2, "{json}");
    assert!(counts.iter().all(|&n| n > 0), "{json}");
    let (min, max) = (counts.iter().min().unwrap(), counts.iter().max().unwrap());
    assert!(
        json.contains(&format!(
            "\"instructions\": {{ \"min\": {min}, \"median\": {min}, \"max\": {max} }}"
        )),
        "{json}"
    );
}

/// A simulated host with no PMC access (a hosted macOS runner): the fake `time -l` leaves out the
/// "instructions retired" line. ep9-bench must say so in the JSON and exit 3, not fail with a
/// parse error.
#[cfg(unix)]
#[test]
fn no_counter_is_explicit() {
    let fake = fake_time_no_pmc();
    let out = bench(
        &[
            "--counter",
            "time",
            "--runs",
            "2",
            "--warmup",
            "1",
            "--",
            BIN,
            "--help",
        ],
        Some(&fake),
    );
    assert_no_counter(&out);
    let json = String::from_utf8_lossy(&out.stdout);
    // The note is the probe's parse result, not a failure to start the fake.
    assert!(
        json.contains("no instruction count in the macos-time-l-instructions-retired output"),
        "{json}"
    );
    assert!(json.contains("only for a count above 0"), "{json}");
}

#[test]
fn failing_command_is_an_error() {
    // `ep9-bench --no-such-flag` exits 2 on every host. It is an error with or without a counter.
    let out = bench(&["--runs", "1", "--", BIN, "--no-such-flag"], None);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("exited with"), "{stderr}");
}

/// The command after `--` gets its own `--help`; ep9-bench does not print its usage.
#[test]
fn help_after_separator_goes_to_the_command() {
    let out = bench(&["--runs", "1", "--warmup", "0", "--", BIN, "--help"], None);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"schema\": \"ep9-bench/v1\""), "{stdout}");
    assert!(!stdout.contains("usage: ep9-bench"), "{stdout}");
}
