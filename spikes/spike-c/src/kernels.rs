//! The protocol kernels (docs/measurements/2026-10-27-spike-c-protocol.md §2): which v8-v7
//! file, which hot functions Spike C compiles, and the driver that runs a kernel in one
//! script and prints a line that shows the run was right.
//!
//! The files are read from the pinned checkout `third_party/bench-v8v7` (pin
//! `third_party/bench-v8v7.commit`; `bash scripts/sync-bench.sh bench-v8v7` makes it), or from
//! the directory in `SPIKE_C_BENCH_DIR`. Either way each file must have the git blob id it has
//! at the pin ([`PINNED`]; `git hash-object` of the bytes read), so a changed kernel is
//! refused, not run as the pinned one. The driver uses the stub harness of
//! `tools/bench/cross/v8v7.mjs` (decision PERF-2): the kernel file as it is, a stub
//! `BenchmarkSuite`/`Benchmark`, and base.js's deterministic `Math.random` block, copied from
//! the pinned base.js at run time.
//!
//! ```text
//!  driver script = stub harness + Math.random (base.js) + <kernel>.js + loop
//!    loop: Setup(); hook(); run() n times; print(check()); TearDown(); print(name + " " + n)
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The git blob id of each file the drivers read, at the pin `third_party/bench-v8v7.commit`
/// (`git rev-parse <pin>:bench/bench-v8/<file>`; the test `pinned_blobs_are_the_pin` checks
/// them against the checkout).
pub const PINNED: [(&str, &str); 4] = [
    ("base.js", "7964a4cb37dff3beb9ac18b1d43ffff9b862be1f"),
    ("crypto.js", "a15aa80d773d54569e074792d1f04f3d678e4783"),
    (
        "navier-stokes.js",
        "27c0403a7a5f2287c26fc07cee66621007787b64",
    ),
    ("richards.js", "2308066637c78ea4138cb23769615b01c2a64f98"),
];

/// The navier-stokes check line after one frame (n = 1): node v24.22.1 printed it for
/// `spike-c driver navier-stokes --n 1` on 2026-10-26 (the protocol's tier-differential
/// reference for kernel 2; §5). Its form is that of [`NS_CHECK`].
pub const NAVIER_STOKES_CHECK_N1: &str = "checksum 328.09114083768867 -376.62445547361995 \
     -376.82420862988374 digest 104004329 1915391150";

/// The part a kernel plays in the Spike C gate (protocol §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// One of the 2 gate kernels.
    Gate,
    /// The named third candidate (a drop of one gate kernel still leaves 2).
    Third,
}

/// One hot function of a kernel.
#[derive(Clone, Copy, Debug)]
pub struct Hot {
    /// The name in reports (`Scheduler.prototype.schedule`).
    pub name: &'static str,
    /// First and last line in the kernel file (the pinned v8-v7 file).
    pub lines: (u32, u32),
    /// A JS expression that gives the function value after the driver ran, or `None` if no
    /// expression reaches it (an inner closure: install needs a lookup by function id).
    pub reach: Option<&'static str>,
    /// Share of the kernel's self time under `node --jitless`, percent: the lowest of the
    /// samples in protocol §2.1 (est.; an ep9 T0 share is not measured yet).
    pub jitless_share: f32,
}

/// One protocol kernel.
#[derive(Clone, Copy, Debug)]
pub struct Kernel {
    pub name: &'static str,
    /// The file in `bench/bench-v8`.
    pub file: &'static str,
    pub role: Role,
    /// The hot set, hottest first. The first entry is the kernel's first compile target.
    pub hot: &'static [Hot],
    /// `run()` calls per driver run in the timing (the `tools/bench/cross/v8v7.mjs` N).
    pub n: u32,
    /// JS run after Setup (may be empty).
    pub hook: &'static str,
    /// A JS expression whose value is printed after the runs ("" = none; the kernel checks
    /// itself and throws on a wrong result).
    pub check: &'static str,
    /// The check line node prints at n = 1 ("" = no check, or no reference).
    pub check_n1: &'static str,
}

/// navier-stokes has no result check in v7: the driver keeps the `Field` of the last frame and
/// prints a check line of its density and velocity fields.
const NS_HOOK: &str = "var __spikeCField = null;\n\
solver.setDisplayFunction(function (f) { __spikeCField = f; });";
/// The navier-stokes check line: `checksum <d> <u> <v> digest <ha> <hb>`, over every cell of the
/// three fields (the boundary cells too: x and y from -1 to the size), in index order.
///
/// ```text
///  each cell, density then x velocity then y velocity:
///    a number that is not finite, or not a number  -> counted; any such cell: "checksum bad <count>"
///    else its text ("-0" for -0, else String(c)), then ","  -> both digests
///  d, u, v   the plain sums (a reader's check only: a swap of two cells keeps them)
///  ha        FNV-1a 32 of that text  (ha * 16777619 mod 2^32, as (ha << 24) + ha * 403)
///  hb        hb * 1000003 + char mod 4294967291
/// ```
///
/// The text of a finite double is exact (the shortest form that reads back as the same
/// double), so the digests see every cell's value and position (Codex r3: the sums alone kept
/// a swap of two unequal cells). Every step is exact in doubles (no `Math.imul`: T0 has none
/// at the base). The line is made after the `run()` loop, outside the timed region (protocol §3).
pub const NS_CHECK: &str = "(function (f) {\n\
  var w = f.width(), h = f.height(), d = 0, u = 0, v = 0, ha = 2166136261, hb = 0, bad = 0;\n\
  function cell(c) {\n\
    if (typeof c !== 'number' || c - c !== 0) { bad++; return 0; }\n\
    var s = c === 0 && 1 / c < 0 ? '-0' : '' + c;\n\
    for (var j = 0; j <= s.length; j++) {\n\
      var k = j < s.length ? s.charCodeAt(j) : 44;\n\
      ha = (ha ^ k) >>> 0;\n\
      ha = (((ha << 24) >>> 0) + ha * 403) % 4294967296;\n\
      hb = (hb * 1000003 + k) % 4294967291;\n\
    }\n\
    return c;\n\
  }\n\
  for (var y = -1; y <= h; y++) for (var x = -1; x <= w; x++) {\n\
    d += cell(f.getDensity(x, y)); u += cell(f.getXVelocity(x, y)); v += cell(f.getYVelocity(x, y));\n\
  }\n\
  if (bad) return 'checksum bad ' + bad;\n\
  return 'checksum ' + d + ' ' + u + ' ' + v + ' digest ' + ha + ' ' + hb;\n\
})(__spikeCField)";

/// The kernels of the protocol (§2). The line numbers are those of the pinned files.
pub const KERNELS: [Kernel; 3] = [
    Kernel {
        name: "crypto",
        file: "crypto.js",
        role: Role::Gate,
        hot: &[
            Hot {
                name: "am3",
                lines: (101, 115),
                reach: Some("am3"),
                jitless_share: 48.1,
            },
            Hot {
                name: "montReduce",
                lines: (576, 593),
                reach: Some("montReduce"),
                jitless_share: 21.8,
            },
            Hot {
                name: "bnpSquareTo",
                lines: (424, 441),
                reach: Some("bnpSquareTo"),
                jitless_share: 5.2,
            },
        ],
        n: 8,
        hook: "",
        check: "",
        check_n1: "",
    },
    Kernel {
        name: "navier-stokes",
        file: "navier-stokes.js",
        role: Role::Gate,
        hot: &[
            Hot {
                name: "lin_solve",
                lines: (122, 149),
                reach: None,
                jitless_share: 43.1,
            },
            Hot {
                name: "project",
                lines: (233, 269),
                reach: None,
                jitless_share: 15.5,
            },
            Hot {
                name: "advect",
                lines: (198, 231),
                reach: None,
                jitless_share: 5.5,
            },
        ],
        n: 12,
        hook: NS_HOOK,
        check: NS_CHECK,
        check_n1: NAVIER_STOKES_CHECK_N1,
    },
    Kernel {
        name: "richards",
        file: "richards.js",
        role: Role::Third,
        hot: &[
            Hot {
                name: "Scheduler.prototype.schedule",
                lines: (183, 193),
                reach: Some("Scheduler.prototype.schedule"),
                jitless_share: 34.7,
            },
            Hot {
                name: "TaskControlBlock.prototype.run",
                lines: (319, 333),
                reach: Some("TaskControlBlock.prototype.run"),
                jitless_share: 12.0,
            },
        ],
        n: 200,
        hook: "",
        check: "",
        check_n1: "",
    },
];

/// The kernel named `name`.
pub fn kernel(name: &str) -> Option<&'static Kernel> {
    KERNELS.iter().find(|k| k.name == name)
}

/// The directory of the v8-v7 files: `SPIKE_C_BENCH_DIR`, else the pinned checkout.
pub fn bench_dir() -> PathBuf {
    match std::env::var_os("SPIKE_C_BENCH_DIR") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../third_party/bench-v8v7/bench/bench-v8"),
    }
}

/// True if the v8-v7 files are there (tests that need them skip, with a line, if not).
pub fn bench_present() -> bool {
    bench_dir().join("base.js").is_file()
}

/// The git blob id of `bytes` (`git hash-object --no-filters --stdin`).
fn blob_id(bytes: &[u8]) -> Result<String, String> {
    let mut child = Command::new("git")
        .args(["hash-object", "--no-filters", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git hash-object: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("git hash-object: no stdin")?;
    stdin
        .write_all(bytes)
        .map_err(|e| format!("git hash-object: {e}"))?;
    drop(stdin);
    let out = child
        .wait_with_output()
        .map_err(|e| format!("git hash-object: {e}"))?;
    if !out.status.success() {
        return Err(format!("git hash-object: {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The text of `file` in `dir`, refused unless its blob id is the pinned one ([`PINNED`]).
fn read(dir: &Path, file: &str) -> Result<String, String> {
    let p = dir.join(file);
    let bytes = std::fs::read(&p).map_err(|e| {
        format!(
            "{}: {e} (run `bash scripts/sync-bench.sh bench-v8v7`, or set SPIKE_C_BENCH_DIR)",
            p.display()
        )
    })?;
    let want = PINNED
        .iter()
        .find(|(f, _)| *f == file)
        .ok_or_else(|| format!("{file}: no pinned blob id (kernels::PINNED)"))?
        .1;
    let got = blob_id(&bytes)?;
    if got != want {
        return Err(format!(
            "{}: blob {got}, not the pinned blob {want} (third_party/bench-v8v7.commit)",
            p.display()
        ));
    }
    String::from_utf8(bytes).map_err(|e| format!("{}: {e}", p.display()))
}

/// base.js's `Math.random = (function() { ... })();` block (the line that starts it through
/// the line `})();`), as `tools/bench/cross/v8v7.mjs` `mathRandom` takes it.
pub fn math_random(base: &str) -> Result<String, String> {
    let start = base
        .find("\nMath.random = (function() {\n")
        .ok_or("base.js: no \"Math.random = (function() {\" block")?
        + 1;
    let end = base[start..]
        .find("\n})();")
        .ok_or("base.js: the Math.random block has no end")?
        + start
        + "\n})();".len();
    Ok(base[start..end].to_string())
}

/// The stub harness (decision PERF-2), as `tools/bench/cross/v8v7.mjs` makes it, with its
/// `print` line for node (`PRINT_PRELUDE`), so node runs the same text.
fn stub(math_random: &str) -> String {
    format!(
        "if (typeof print !== \"function\") globalThis.print = (...a) => console.log(...a);\n\
         var __ep9Benchmarks = [];\n\
         function BenchmarkSuite(name, reference, benchmarks) {{\n\
         \x20 for (var i = 0; i < benchmarks.length; i++) __ep9Benchmarks.push(benchmarks[i]);\n\
         }}\n\
         function Benchmark(name, run, setup, tearDown) {{\n\
         \x20 this.name = name;\n\
         \x20 this.run = run;\n\
         \x20 this.Setup = setup ? setup : function() {{ }};\n\
         \x20 this.TearDown = tearDown ? tearDown : function() {{ }};\n\
         }}\n\
         if (typeof Math === \"object\") {{\n\
         {math_random}\n\
         }}\n"
    )
}

/// A driver script for one kernel.
#[derive(Clone, Debug)]
pub struct Driver {
    pub text: String,
    /// Line L of the kernel file is line L + `offset` of `text`.
    pub offset: u32,
    /// The line `<benchmark name> <n>` of each benchmark, in order. A right run prints, per
    /// benchmark, the check line (if `has_check`) and then this line.
    pub expect: Vec<String>,
    pub has_check: bool,
    /// The check line a right run prints, where a reference is known (n = 1,
    /// [`Kernel::check_n1`]).
    pub check_ref: Option<&'static str>,
}

impl Driver {
    /// True if `output` is what a right run prints: per benchmark, its check line (if
    /// `has_check`) then `<name> <n>`, and nothing else. A check line must be there; it
    /// must equal `check_ref` when that is known, and else have the form of [`NS_CHECK`]:
    /// `checksum`, three finite numbers, `digest` and two 32-bit integers (the values are
    /// compared with the T0 run in [`Driver::matches_t0`], not here).
    pub fn matches(&self, output: &[String]) -> bool {
        let per = if self.has_check { 2 } else { 1 };
        output.len() == self.expect.len() * per
            && output.chunks(per).zip(&self.expect).all(|(lines, name)| {
                lines[per - 1] == *name && (!self.has_check || self.check_ok(&lines[0]))
            })
    }

    /// True if `output` is a right run ([`Driver::matches`]) and equals the T0 run `t0` of the
    /// same driver line for line: the check line holds the cell digests, so a compiled run with
    /// any cell changed or moved differs (protocol §3, §5).
    pub fn matches_t0(&self, t0: &[String], output: &[String]) -> bool {
        self.matches(t0) && self.matches(output) && output == t0
    }

    fn check_ok(&self, line: &str) -> bool {
        if let Some(r) = self.check_ref {
            return line == r;
        }
        let Some(values) = line.strip_prefix("checksum ") else {
            return false;
        };
        let values: Vec<&str> = values.split(' ').collect();
        values.len() == 6
            && values[..3]
                .iter()
                .all(|v| v.parse::<f64>().is_ok_and(f64::is_finite))
            && values[3] == "digest"
            && values[4..].iter().all(|v| v.parse::<u32>().is_ok())
    }
}

/// The names of the benchmarks a kernel file registers (`new Benchmark("Name", ...)`).
pub fn benchmark_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("new Benchmark(") {
        rest = &rest[i + "new Benchmark(".len()..];
        let r = rest.trim_start();
        if let Some(q) = r.chars().next().filter(|c| *c == '"' || *c == '\'')
            && let Some(end) = r[1..].find(q)
        {
            names.push(r[1..1 + end].to_string());
        }
    }
    names
}

/// The driver of `k` with `n` runs per benchmark, from the files in [`bench_dir`].
pub fn driver(k: &Kernel, n: u32) -> Result<Driver, String> {
    driver_in(&bench_dir(), k, n)
}

/// The driver of `k` with `n` runs per benchmark, from the files in `dir` (each must be the
/// pinned blob).
pub fn driver_in(dir: &Path, k: &Kernel, n: u32) -> Result<Driver, String> {
    let base = read(dir, "base.js")?;
    let src = read(dir, k.file)?;
    let names = benchmark_names(&src);
    if names.is_empty() {
        return Err(format!("{}: no \"new Benchmark(\" call", k.file));
    }
    let before = format!(
        "// Spike C driver: {} (v8-v7, pin third_party/bench-v8v7.commit), n = {n}\n{}// ---- {} ----\n",
        k.name,
        stub(&math_random(&base)?),
        k.file
    );
    let check = if k.check.is_empty() {
        String::new()
    } else {
        format!("    print({});\n", k.check)
    };
    let hook = if k.hook.is_empty() {
        String::new()
    } else {
        format!("    {}\n", k.hook.replace('\n', "\n    "))
    };
    let after = format!(
        "// ---- end of {} ----\n\
         (function () {{\n\
         \x20 for (var j = 0; j < __ep9Benchmarks.length; j++) {{\n\
         \x20   var b = __ep9Benchmarks[j];\n\
         \x20   b.Setup();\n\
         {hook}\
         \x20   for (var i = 0; i < {n}; i++) b.run();\n\
         {check}\
         \x20   b.TearDown();\n\
         \x20   print(b.name + \" {n}\");\n\
         \x20 }}\n\
         }})();\n",
        k.file
    );
    let offset = before.matches('\n').count() as u32;
    let sep = if src.ends_with('\n') { "" } else { "\n" };
    Ok(Driver {
        text: format!("{before}{src}{sep}{after}"),
        offset,
        expect: names.iter().map(|x| format!("{x} {n}")).collect(),
        has_check: !k.check.is_empty(),
        check_ref: (n == 1 && !k.check_n1.is_empty()).then_some(k.check_n1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Session;
    use spike_b::harness::Config;

    #[test]
    fn names_and_math_random() {
        assert_eq!(
            benchmark_names("x = [new Benchmark(\"Encrypt\", e), new Benchmark( 'Decrypt', d)];"),
            ["Encrypt", "Decrypt"]
        );
        let base = "a\nMath.random = (function() {\n  var s = 1;\n})();\nb\n";
        assert_eq!(
            math_random(base).expect("block"),
            "Math.random = (function() {\n  var s = 1;\n})();"
        );
        assert!(math_random("Math.random = 1;").is_err());
    }

    /// Each kernel's hot functions are on the lines the table names, in the pinned file, and
    /// the driver runs the kernel in T0 (n = 1) with the expected output. Skips (with a line)
    /// without the v8-v7 checkout.
    #[test]
    fn kernels_run_in_t0_and_hot_lines_hold() {
        if !bench_present() {
            eprintln!(
                "skip: no {} (scripts/sync-bench.sh bench-v8v7)",
                bench_dir().display()
            );
            return;
        }
        for k in &KERNELS {
            let d = driver(k, 1).expect("driver");
            let mut s = Session::new(&Config::default()).expect("session");
            let r = s.run(&d.text, "driver.js");
            assert_eq!(
                r.result,
                Ok("undefined".to_string()),
                "{}: {:?}",
                k.name,
                r.output
            );
            assert!(
                d.matches(&r.output),
                "{}: {:?} vs {:?}",
                k.name,
                r.output,
                d.expect
            );
            let script = s.script("driver.js").expect("script");
            let lines: Vec<&str> = script.source.lines().collect();
            for h in k.hot {
                let first = h.lines.0 + d.offset;
                let ids = script.functions_on_line(first);
                assert_eq!(ids.len(), 1, "{} {}: line {first}: {ids:?}", k.name, h.name);
                // The function's source range ends on the table's last line.
                let end = script.unit.unit().functions[ids[0] as usize].source_end as usize;
                let last = script.source[..end].matches('\n').count() as u32 + 1;
                assert_eq!(
                    last,
                    h.lines.1 + d.offset,
                    "{} {}: last line",
                    k.name,
                    h.name
                );
                let tail = h.name.rsplit('.').next().expect("name");
                assert!(
                    lines[(first - 1) as usize].contains(tail),
                    "{} {}",
                    k.name,
                    h.name
                );
            }
            for h in k.hot {
                if let Some(expr) = h.reach {
                    let f = s.function_value(expr).expect("value");
                    assert!(s.input_of(f).is_ok(), "{} {expr}", k.name);
                }
            }
        }
    }

    /// The navier-stokes checksum after one frame in T0 equals node's
    /// ([`NAVIER_STOKES_CHECK_N1`]).
    #[test]
    fn navier_stokes_checksum_equals_node() {
        if !bench_present() {
            eprintln!(
                "skip: no {} (scripts/sync-bench.sh bench-v8v7)",
                bench_dir().display()
            );
            return;
        }
        let k = kernel("navier-stokes").expect("kernel");
        let d = driver(k, 1).expect("driver");
        let mut s = Session::new(&Config::default()).expect("session");
        let r = s.run(&d.text, "driver.js");
        assert_eq!(r.output, [NAVIER_STOKES_CHECK_N1, "NavierStokes 1"]);
        assert!(d.matches(&r.output));
    }

    /// The fields of a check line after `checksum`.
    fn fields(line: &str) -> Vec<&str> {
        line.split(' ').skip(1).collect()
    }

    /// Negative controls for the navier-stokes check (Codex r3: the three sums alone kept a
    /// swap of the unequal density cells (58,0) and (59,0)). Each control changes the last
    /// frame's fields just before the check line; the true run passes, each changed run fails,
    /// both against node's line (n = 1) and against the T0 run (the rule for any n).
    #[test]
    fn navier_stokes_check_sees_each_cell() {
        if !bench_present() {
            eprintln!(
                "skip: no {} (scripts/sync-bench.sh bench-v8v7)",
                bench_dir().display()
            );
            return;
        }
        let k = kernel("navier-stokes").expect("kernel");
        let d = driver(k, 1).expect("driver");
        let at = "    print((function (f) {";
        let run = |js: &str| {
            let text = d.text.replacen(at, &format!("    {js}\n{at}"), 1);
            assert_ne!(text, d.text, "no check line in the driver");
            let mut s = Session::new(&Config::default()).expect("session");
            let r = s.run(&text, "driver.js");
            assert_eq!(
                r.result,
                Ok("undefined".to_string()),
                "{js}: {:?}",
                r.output
            );
            r.output
        };
        let right = run("");
        assert!(d.matches(&right));
        let no_ref = Driver {
            check_ref: None,
            ..d.clone()
        };
        assert!(no_ref.matches_t0(&right, &right));

        let swap = run(
            "(function (f) { var a = f.getDensity(58, 0), b = f.getDensity(59, 0); \
             if (a === b) throw new Error('equal cells'); \
             f.setDensity(58, 0, b); f.setDensity(59, 0, a); })(__spikeCField);",
        );
        // The sums stay (the old check passed this run); the digests do not.
        assert_eq!(fields(&swap[0])[..3], fields(&right[0])[..3]);
        assert_ne!(swap[0], right[0]);
        assert!(!d.matches(&swap), "{swap:?}");
        assert!(no_ref.matches(&swap), "the form is right: {swap:?}");
        assert!(!no_ref.matches_t0(&right, &swap));

        // -0 in a 0 cell: the sums stay, the cell text differs.
        let neg_zero = run("__spikeCField.setDensity(58, 0, -0);");
        assert_eq!(fields(&neg_zero[0])[..3], fields(&right[0])[..3]);
        assert!(!d.matches(&neg_zero) && !no_ref.matches_t0(&right, &neg_zero));

        // A non-finite cell is refused, with or without a reference.
        for bad in ["NaN", "Infinity", "undefined", "'0'"] {
            let out = run(&format!(
                "__spikeCField.setVelocity(10, 20, {bad}, __spikeCField.getYVelocity(10, 20));"
            ));
            assert_eq!(out[0], "checksum bad 1", "{bad}");
            assert!(!d.matches(&out) && !no_ref.matches(&out), "{bad}");
        }
    }

    fn driver_with_check(check_ref: Option<&'static str>) -> Driver {
        Driver {
            text: String::new(),
            offset: 0,
            expect: vec!["NavierStokes 1".into()],
            has_check: true,
            check_ref,
        }
    }

    fn lines(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    /// A missing, malformed, non-finite or misplaced check line is not a right run (Codex r1:
    /// the old predicate dropped every `checksum` line and accepted all four).
    #[test]
    fn matches_needs_the_check_line() {
        let d = driver_with_check(None);
        let good = "checksum 1.5 -2 3e-7 digest 0 4294967295";
        assert!(d.matches(&lines(&[good, "NavierStokes 1"])));
        for bad in [
            &["NavierStokes 1"][..],
            &["checksum NaN NaN NaN digest 1 2", "NavierStokes 1"],
            &["checksum 1 2 Infinity digest 1 2", "NavierStokes 1"],
            &["checksum 1 2 3", "NavierStokes 1"],
            &["checksum bad 1", "NavierStokes 1"],
            &["checksum 1 2 3 digest 1.5 2", "NavierStokes 1"],
            &["checksum 1 2 3 digest 1 -2", "NavierStokes 1"],
            &["checksum 1 2 3 digest 1 4294967296", "NavierStokes 1"],
            &["checksum 1 2 3 sums 1 2", "NavierStokes 1"],
            &["NavierStokes 1", good],
            &[good, good, "NavierStokes 1"],
        ] {
            assert!(!d.matches(&lines(bad)), "{bad:?}");
        }
        let d = driver_with_check(Some(NAVIER_STOKES_CHECK_N1));
        assert!(d.matches(&lines(&[NAVIER_STOKES_CHECK_N1, "NavierStokes 1"])));
        assert!(!d.matches(&lines(&[good, "NavierStokes 1"])));
        // The round-1 line (sums only) is not node's line any more.
        assert!(!d.matches(&lines(&[
            "checksum 320.5614702136357 -372.6606391856326 -366.08057515126234",
            "NavierStokes 1"
        ])));
        let d = Driver {
            has_check: false,
            check_ref: None,
            expect: vec!["Encrypt 1".into(), "Decrypt 1".into()],
            ..driver_with_check(None)
        };
        assert!(d.matches(&lines(&["Encrypt 1", "Decrypt 1"])));
        assert!(!d.matches(&lines(&["Encrypt 1"])));
        assert!(!d.matches(&lines(&["checksum 1 2 3", "Encrypt 1", "Decrypt 1"])));
    }

    /// [`PINNED`] holds the blob ids of the pin in `third_party/bench-v8v7.commit`. Skips
    /// (with a line) without the git checkout.
    #[test]
    fn pinned_blobs_are_the_pin() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let checkout = root.join("third_party/bench-v8v7");
        if !checkout.join("bench/bench-v8/base.js").is_file() {
            eprintln!("skip: no {} checkout", checkout.display());
            return;
        }
        let pin =
            std::fs::read_to_string(root.join("third_party/bench-v8v7.commit")).expect("pin file");
        for (file, blob) in PINNED {
            let out = Command::new("git")
                .arg("-C")
                .arg(&checkout)
                .args([
                    "rev-parse",
                    &format!("{}:bench/bench-v8/{file}", pin.trim()),
                ])
                .output()
                .expect("git");
            assert!(out.status.success(), "{file}: {out:?}");
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), blob, "{file}");
        }
    }

    /// A copy of the files equal to the pin gives the same driver; a changed kernel (its
    /// benchmark `run` made a no-op, the hot lines kept) is refused (Codex r1: it ran and
    /// passed as the pinned kernel).
    #[test]
    fn a_changed_kernel_is_refused() {
        if !bench_present() {
            eprintln!(
                "skip: no {} (scripts/sync-bench.sh bench-v8v7)",
                bench_dir().display()
            );
            return;
        }
        let dir = std::env::temp_dir().join(format!("spike-c-pin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        for (file, _) in PINNED {
            std::fs::copy(bench_dir().join(file), dir.join(file)).expect("copy");
        }
        let k = kernel("crypto").expect("kernel");
        let same = driver_in(&dir, k, 1).map(|d| d.text);
        let pinned = driver(k, 1).map(|d| d.text);
        let src = std::fs::read_to_string(dir.join("crypto.js")).expect("read");
        let changed = src.replace(
            "new Benchmark(\"Encrypt\", encrypt)",
            "new Benchmark(\"Encrypt\", function(){})",
        );
        assert_ne!(changed, src);
        std::fs::write(dir.join("crypto.js"), changed).expect("write");
        let refused = driver_in(&dir, k, 1);
        std::fs::remove_dir_all(&dir).expect("clean up");
        assert_eq!(same, pinned);
        let e = refused.expect_err("a changed crypto.js");
        assert!(e.contains("not the pinned blob"), "{e}");
    }
}
