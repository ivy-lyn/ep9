//! Instruction counters. Each one wraps the benchmarked command in an OS tool and parses the
//! count from the tool's output file, so the command's own stdout and stderr stay apart.
//!
//! | Counter      | Tool                                    | OS    | Counts                      |
//! |--------------|-----------------------------------------|-------|-----------------------------|
//! | `time`       | `/usr/bin/time -l` "instructions retired" | macOS | all retired instructions    |
//! | `perf`       | `perf stat -e instructions:u`           | Linux | user-mode instructions      |
//! | `cachegrind` | `valgrind --tool=cachegrind` "I refs"   | Linux | user-mode instructions (VM) |
//!
//! `cachegrind` is the fallback for Linux hosts with no hardware counters (many cloud VMs).
//!
//! Before the runs, [`Counter::probe`] checks that the counter gives a count on this host. Some
//! hosts have no counter: Windows, and a VM without PMC access, such as a hosted macOS runner
//! (Apple's `time.c` prints "instructions retired" only for a count above 0). On such a host the
//! runs go without a wrapper and give wall time only (`instructions: None`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

/// The program that a probe runs.
const PROBE_PROGRAM: &str = if cfg!(target_os = "macos") {
    "/usr/bin/true"
} else {
    "/bin/true"
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counter {
    Time,
    Perf,
    Cachegrind,
}

impl Counter {
    pub fn parse(s: &str) -> Option<Counter> {
        match s {
            "time" => Some(Counter::Time),
            "perf" => Some(Counter::Perf),
            "cachegrind" => Some(Counter::Cachegrind),
            _ => None,
        }
    }

    /// The name written to the JSON report.
    pub fn id(self) -> &'static str {
        match self {
            Counter::Time => "macos-time-l-instructions-retired",
            Counter::Perf => "perf-stat-instructions-u",
            Counter::Cachegrind => "cachegrind-i-refs",
        }
    }

    /// Checks that this counter gives a count on this host: one run of `true` under it must give
    /// a count above 0. `Err` tells why it does not.
    pub fn probe(self) -> Result<(), String> {
        let why = match measure(Some(self), Path::new(PROBE_PROGRAM), &[], false) {
            Ok(Sample {
                instructions: Some(n),
                ..
            }) if n > 0 => return Ok(()),
            Ok(_) => "the count was 0".to_string(),
            Err(e) => e,
        };
        Err(format!(
            "{}: {why} (probe run: {PROBE_PROGRAM}). {}",
            self.id(),
            self.no_count_hint()
        ))
    }

    fn no_count_hint(self) -> &'static str {
        match self {
            Counter::Time => {
                "macOS `time -l` prints \"instructions retired\" only for a count above 0, so \
                 this host gives no instruction count (for example a VM without PMC access, such \
                 as a hosted macOS runner)"
            }
            Counter::Perf => {
                "perf is missing, or the host hides hardware counters (VM) or forbids them \
                 (kernel.perf_event_paranoid)"
            }
            Counter::Cachegrind => "valgrind is missing or does not run",
        }
    }

    /// Picks the counter for this host: `time` on macOS; on Linux `perf`, else `cachegrind`. Each
    /// one must pass [`Counter::probe`]. `Err` tells why no counter gives a count.
    pub fn detect() -> Result<Counter, String> {
        let candidates: &[Counter] = match std::env::consts::OS {
            "macos" => &[Counter::Time],
            "linux" => &[Counter::Perf, Counter::Cachegrind],
            os => return Err(format!("ep9-bench has no instruction counter for {os}")),
        };
        let mut why = Vec::new();
        for &c in candidates {
            match c.probe() {
                Ok(()) => return Ok(c),
                Err(e) => why.push(e),
            }
        }
        Err(why.join("; "))
    }

    /// The wrapper command, before the benchmarked program. It writes its report to `report`.
    fn wrapper(self, report: &Path) -> Command {
        match self {
            Counter::Time => {
                let mut c = Command::new(time_tool());
                c.arg("-l").arg("-o").arg(report);
                c
            }
            Counter::Perf => {
                let mut c = Command::new("perf");
                c.args(["stat", "-x", ",", "-e", "instructions:u", "-o"]);
                c.arg(report).arg("--");
                c
            }
            Counter::Cachegrind => {
                let mut c = Command::new("valgrind");
                c.args([
                    "--tool=cachegrind",
                    "--cache-sim=no",
                    "--cachegrind-out-file=/dev/null",
                ]);
                c.arg(format!("--log-file={}", report.display()));
                c.arg("--");
                c
            }
        }
    }
}

/// `/usr/bin/time`, or the program in `EP9_BENCH_TIME` (tests set a fake: tests/testdata).
fn time_tool() -> PathBuf {
    std::env::var_os("EP9_BENCH_TIME").map_or_else(|| PathBuf::from("/usr/bin/time"), PathBuf::from)
}

/// One measured run.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// `None` when the run has no counter (the host has none).
    pub instructions: Option<u64>,
    pub wall_ns: u128,
    /// Peak RSS in bytes, when the tool reports it (`time` only).
    pub max_rss_bytes: Option<u64>,
}

/// Runs `program args` once, under `counter` if there is one; with `None` it measures wall time
/// only. A nonzero exit of the program is an error. With a counter, a run that gives no count is
/// an error too.
pub fn measure(
    counter: Option<Counter>,
    program: &Path,
    args: &[String],
    show_output: bool,
) -> Result<Sample, String> {
    let report = temp_path();
    let mut cmd = match counter {
        Some(c) => {
            let mut cmd = c.wrapper(&report);
            cmd.arg(program);
            cmd
        }
        None => Command::new(program),
    };
    cmd.args(args);
    if !show_output {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let start = Instant::now();
    let status = cmd.status().map_err(|e| match counter {
        Some(c) => format!("cannot run the {} counter: {e}", c.id()),
        None => format!("cannot run {}: {e}", program.display()),
    });
    let wall_ns = start.elapsed().as_nanos();
    let text = fs::read_to_string(&report).unwrap_or_default();
    let _ = fs::remove_file(&report);
    let status = status?;
    if !status.success() {
        return Err(format!("{} exited with {status}", program.display()));
    }
    let Some(counter) = counter else {
        return Ok(Sample {
            instructions: None,
            wall_ns,
            max_rss_bytes: None,
        });
    };
    let instructions = match counter {
        Counter::Time => parse_time_l(&text),
        Counter::Perf => parse_perf_csv(&text),
        Counter::Cachegrind => parse_cachegrind(&text),
    }
    .ok_or_else(|| format!("no instruction count in the {} output", counter.id()))?;
    let max_rss_bytes = match counter {
        Counter::Time => time_l_field(&text, "maximum resident set size"),
        _ => None,
    };
    Ok(Sample {
        instructions: Some(instructions),
        wall_ns,
        max_rss_bytes,
    })
}

/// A unique report path in the temp dir.
fn temp_path() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ep9-bench-{}-{n}.txt", std::process::id()))
}

/// macOS `/usr/bin/time -l` lines look like `   10906475  instructions retired`.
fn time_l_field(text: &str, name: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_suffix(name)?;
        rest.trim().parse().ok()
    })
}

pub fn parse_time_l(text: &str) -> Option<u64> {
    time_l_field(text, "instructions retired")
}

/// `perf stat -x,` CSV: `<count>,<unit>,<event>,...`. The count is `<not supported>` or
/// `<not counted>` when the host has no usable counter.
pub fn parse_perf_csv(text: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let mut f = l.split(',');
        let count = f.next()?;
        let _unit = f.next()?;
        let event = f.next()?;
        if event.starts_with("instructions") {
            count.trim().parse().ok()
        } else {
            None
        }
    })
}

/// valgrind log: `==123== I   refs:      1,234,567` (or `I refs:`).
pub fn parse_cachegrind(text: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let (_, rest) = l.split_once(" I")?;
        let rest = rest.trim_start().strip_prefix("refs:")?;
        rest.trim().replace(',', "").parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_l() {
        let text = "        0.00 real         0.00 user         0.00 sys\n\
                    \x20            1261568  maximum resident set size\n\
                    \x20           10906475  instructions retired\n\
                    \x20            8849031  cycles elapsed\n";
        assert_eq!(parse_time_l(text), Some(10_906_475));
        assert_eq!(
            time_l_field(text, "maximum resident set size"),
            Some(1_261_568)
        );
        assert_eq!(parse_time_l("time: command terminated abnormally\n"), None);
        // No PMC access (hosted macOS VM): time.c leaves out the lines with a count of 0.
        let no_pmc = "        0.00 real         0.00 user         0.00 sys\n\
                      \x20            1261568  maximum resident set size\n\
                      \x20            1327344  peak memory footprint\n";
        assert_eq!(parse_time_l(no_pmc), None);
    }

    #[test]
    fn perf_csv() {
        let ok =
            "# started on Mon Oct 20 20:00:00 2026\n\n123456789,,instructions:u,1000,100.00,,\n";
        assert_eq!(parse_perf_csv(ok), Some(123_456_789));
        let no = "<not supported>,,instructions:u,0,100.00,,\n";
        assert_eq!(parse_perf_csv(no), None);
        assert_eq!(
            parse_perf_csv("<not counted>,,instructions:u,0,0.00,,\n"),
            None
        );
    }

    #[test]
    fn cachegrind() {
        let log = "==4242== Cachegrind, a high-precision tracing profiler\n\
                   ==4242== \n\
                   ==4242== I refs:        1,234,567\n";
        assert_eq!(parse_cachegrind(log), Some(1_234_567));
        assert_eq!(parse_cachegrind("==1== I   refs:      42\n"), Some(42));
        assert_eq!(parse_cachegrind("==1== error\n"), None);
    }

    #[test]
    fn counter_names() {
        for c in ["time", "perf", "cachegrind"] {
            assert!(Counter::parse(c).is_some());
        }
        assert_eq!(Counter::parse("cycles"), None);
    }
}
