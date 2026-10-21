//! Instruction counters. Each one wraps the benchmarked command in an OS tool and parses the
//! count from the tool's output file, so the command's own stdout and stderr stay apart. That
//! also lets [`measure`] check the command's stdout under any counter (`--expect-stdout`).
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
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

/// Lines of the command's stderr that a failed run prints when the output is not shown.
const STDERR_TAIL_LINES: usize = 20;

/// Bytes of the command's stdout that a run with an expected output reads; more is a wrong output.
const MAX_STDOUT_BYTES: u64 = 1 << 20;

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
        let why = match measure(Some(self), Path::new(PROBE_PROGRAM), &[], false, None) {
            Ok(Sample {
                instructions: Some(n),
                ..
            }) if n > 0 => return Ok(()),
            Ok(_) => "the count was 0".to_string(),
            Err(e) => e.to_string(),
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

/// Why a run failed.
#[derive(Debug)]
pub enum RunError {
    /// The command did not start or exited nonzero, or a counted run gave no count.
    Failed(String),
    /// The command exited with 0, but its stdout is not the expected output.
    WrongOutput(String),
}

impl From<String> for RunError {
    fn from(e: String) -> RunError {
        RunError::Failed(e)
    }
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Failed(e) | RunError::WrongOutput(e) => f.write_str(e),
        }
    }
}

/// Runs `program args` once, under `counter` if there is one; with `None` it measures wall time
/// only. A nonzero exit of the program is an error. With a counter, a run that gives no count is
/// an error too. Without `show_output`, stdout is discarded and stderr goes to a temp file, so the
/// error of a failed run ends with the last [`STDERR_TAIL_LINES`] lines of its stderr.
///
/// With `expect_stdout`, stdout goes to a temp file instead, and a run whose stdout differs from
/// it is a [`RunError::WrongOutput`]. Both sides are compared after CRLF -> LF and a trim of
/// leading and trailing whitespace: the refusal rule of `cross/run.mjs` (`checkRun`).
pub fn measure(
    counter: Option<Counter>,
    program: &Path,
    args: &[String],
    show_output: bool,
    expect_stdout: Option<&str>,
) -> Result<Sample, RunError> {
    let report = temp_path("txt");
    let mut cmd = match counter {
        Some(c) => {
            let mut cmd = c.wrapper(&report);
            cmd.arg(program);
            cmd
        }
        None => Command::new(program),
    };
    cmd.args(args);
    // The capture file and the handle we read it back from; the child writes through a clone.
    let mut capture = None;
    if !show_output {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        if let Some((path, f)) = capture_file("stderr") {
            match f.try_clone() {
                Ok(w) => {
                    cmd.stderr(w);
                    capture = Some((path, f));
                }
                Err(_) => {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }
    // The same for stdout, when it is checked. No file means no check: that is an error.
    let mut stdout_capture = None;
    if expect_stdout.is_some() {
        let (path, f) = capture_file("stdout")
            .ok_or_else(|| "cannot make a temp file for the command's stdout".to_string())?;
        match f.try_clone() {
            Ok(w) => {
                cmd.stdout(w);
                stdout_capture = Some((path, f));
            }
            Err(e) => {
                let _ = fs::remove_file(&path);
                return Err(format!("cannot capture the command's stdout: {e}").into());
            }
        }
    }
    let start = Instant::now();
    let status = cmd.status().map_err(|e| match counter {
        Some(c) => format!("cannot run the {} counter: {e}", c.id()),
        None => format!("cannot run {}: {e}", program.display()),
    });
    let wall_ns = start.elapsed().as_nanos();
    let text = fs::read_to_string(&report).unwrap_or_default();
    let _ = fs::remove_file(&report);
    let stderr_tail = capture.and_then(|(path, mut f)| {
        let tail = status
            .as_ref()
            .is_ok_and(|s| !s.success())
            .then(|| last_lines(&mut f, STDERR_TAIL_LINES));
        let _ = fs::remove_file(&path);
        tail
    });
    let stdout = stdout_capture.map(|(path, mut f)| {
        let text = read_capped(&mut f, MAX_STDOUT_BYTES);
        let _ = fs::remove_file(&path);
        text
    });
    let status = status?;
    if !status.success() {
        let mut e = format!("{} exited with {status}", program.display());
        match stderr_tail {
            Some(tail) if tail.is_empty() => e.push_str(". Its stderr was empty."),
            Some(tail) => {
                e.push_str(". Its stderr ends with:");
                for line in tail {
                    e.push_str("\n  ");
                    e.push_str(&line);
                }
            }
            None => {}
        }
        return Err(e.into());
    }
    if let (Some(want), Some(got)) = (expect_stdout, stdout) {
        let got =
            got.map_err(|e| format!("cannot read the stdout of {}: {e}", program.display()))?;
        check_stdout(program, got.as_deref(), want)?;
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

/// A unique path in the temp dir, with extension `ext`.
fn temp_path(ext: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ep9-bench-{}-{n}.{ext}", std::process::id()))
}

/// A new temp file with extension `ext` for the command's stderr or stdout, and its path.
/// `create_new` never opens a file or symlink that is already at the path (the names are
/// predictable); on unix the mode is 0600. A taken name moves on to the next one. `None` when no
/// file can be made.
fn capture_file(ext: &str) -> Option<(PathBuf, fs::File)> {
    for _ in 0..64 {
        let path = temp_path(ext);
        match open_new_private(&path) {
            Ok(f) => return Some((path, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

/// Creates the file at `path` for reading and writing; fails if anything is already there.
fn open_new_private(path: &Path) -> std::io::Result<fs::File> {
    let mut opts = fs::OpenOptions::new();
    opts.read(true).write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)
}

/// The last `n` lines of `f`, without line ends, read from at most its last 64 KiB as of this
/// call: a process that still writes to the file cannot make the read grow. Empty when the file is
/// empty or cannot be read.
fn last_lines(f: &mut fs::File, n: usize) -> Vec<String> {
    const MAX_READ: u64 = 64 << 10;
    let mut bytes = Vec::new();
    let read = f.metadata().and_then(|m| {
        let len = m.len();
        let start = len.saturating_sub(MAX_READ);
        f.seek(SeekFrom::Start(start))?;
        f.take(len - start).read_to_end(&mut bytes)
    });
    if read.is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let skip = lines.len().saturating_sub(n);
    lines[skip..].iter().map(|l| l.to_string()).collect()
}

/// The text of `f` from its start, or `None` when it has more than `max` bytes.
fn read_capped(f: &mut fs::File, max: u64) -> std::io::Result<Option<String>> {
    let mut bytes = Vec::new();
    f.seek(SeekFrom::Start(0))?;
    f.take(max + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= max).then(|| String::from_utf8_lossy(&bytes).into_owned()))
}

/// `Err(WrongOutput)` unless `got` (`None`: more than [`MAX_STDOUT_BYTES`]) equals `want`, both
/// after CRLF -> LF and a trim.
fn check_stdout(program: &Path, got: Option<&str>, want: &str) -> Result<(), RunError> {
    let norm = |s: &str| s.replace("\r\n", "\n").trim().to_string();
    let want = norm(want);
    let got = match got {
        Some(got) if norm(got) == want => return Ok(()),
        Some(got) => format!("{:?}", short(&norm(got))),
        None => format!("more than {MAX_STDOUT_BYTES} bytes"),
    };
    Err(RunError::WrongOutput(format!(
        "{} printed {got}, want {:?}",
        program.display(),
        short(&want)
    )))
}

/// `s` cut to 200 characters for an error message.
fn short(s: &str) -> String {
    match s.char_indices().nth(200) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_string(),
    }
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

    // Review finding: File::create followed a symlink at the predictable path and truncated its
    // target.
    #[cfg(unix)]
    #[test]
    fn capture_file_never_opens_an_existing_path() {
        let dir = std::env::temp_dir().join(format!("ep9-bench-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("victim.txt");
        fs::write(&target, "keep me").unwrap();
        let link = dir.join("capture.stderr");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let e = open_new_private(&link).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep me");
        let fresh = dir.join("fresh.stderr");
        let _ = fs::remove_file(&fresh);
        drop(open_new_private(&fresh).unwrap());
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&fresh).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    // Review finding: a writer that kept appending made the tail read grow without a bound.
    #[test]
    fn tail_read_is_bounded_while_the_file_grows() {
        let (path, mut f) = capture_file("stderr").unwrap();
        let mut w = f.try_clone().unwrap();
        use std::io::Write as _;
        w.write_all(&b"0123456789abcde\n".repeat(16 << 10)).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let chunk = b"appended line after the snapshot\n".repeat(1024);
                while !stop.load(Ordering::Relaxed) {
                    w.write_all(&chunk).unwrap();
                }
            })
        };
        for _ in 0..20 {
            let lines = last_lines(&mut f, usize::MAX);
            let bytes: usize = lines.iter().map(|l| l.len() + 1).sum();
            assert!(bytes <= (64 << 10) + 1, "read {bytes} bytes");
        }
        stop.store(true, Ordering::Relaxed);
        writer.join().unwrap();
        let lines = last_lines(&mut f, 2);
        assert_eq!(lines.len(), 2);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn counter_names() {
        for c in ["time", "perf", "cachegrind"] {
            assert!(Counter::parse(c).is_some());
        }
        assert_eq!(Counter::parse("cycles"), None);
    }
}
