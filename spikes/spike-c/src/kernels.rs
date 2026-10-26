//! The protocol kernels (docs/measurements/2026-10-27-spike-c-protocol.md §2): which v8-v7
//! file, which hot functions Spike C compiles, and the driver that runs a kernel in one
//! script and prints a line that shows the run was right.
//!
//! The files are read from the pinned checkout `third_party/bench-v8v7` (pin
//! `third_party/bench-v8v7.commit`; `bash scripts/sync-bench.sh bench-v8v7` makes it), or from
//! the directory in `SPIKE_C_BENCH_DIR`. The driver uses the stub harness of
//! `tools/bench/cross/v8v7.mjs` (decision PERF-2): the kernel file as it is, a stub
//! `BenchmarkSuite`/`Benchmark`, and base.js's deterministic `Math.random` block, copied from
//! the pinned base.js at run time.
//!
//! ```text
//!  driver script = stub harness + Math.random (base.js) + <kernel>.js + loop
//!    loop: Setup(); hook(); run() n times; print(check()); TearDown(); print(name + " " + n)
//! ```

use std::path::PathBuf;

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
}

/// navier-stokes has no result check in v7: the driver keeps the `Field` of the last frame and
/// prints a checksum of its density and velocity.
const NS_HOOK: &str = "var __spikeCField = null;\n\
solver.setDisplayFunction(function (f) { __spikeCField = f; });";
const NS_CHECK: &str = "(function (f) {\n\
  var d = 0, u = 0, v = 0, w = f.width(), h = f.height();\n\
  for (var y = 0; y < h; y++) for (var x = 0; x < w; x++) {\n\
    d += f.getDensity(x, y); u += f.getXVelocity(x, y); v += f.getYVelocity(x, y);\n\
  }\n\
  return 'checksum ' + d + ' ' + u + ' ' + v;\n\
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

fn read(file: &str) -> Result<String, String> {
    let p = bench_dir().join(file);
    std::fs::read_to_string(&p).map_err(|e| {
        format!(
            "{}: {e} (run `bash scripts/sync-bench.sh bench-v8v7`, or set SPIKE_C_BENCH_DIR)",
            p.display()
        )
    })
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
    /// What a right run prints: the check line (if any), then `<benchmark name> <n>` per
    /// benchmark. The check line's value is not known here; [`Driver::matches`] skips it.
    pub expect: Vec<String>,
    pub has_check: bool,
}

impl Driver {
    /// True if `output` is what a right run prints (the check lines are compared between runs,
    /// not here).
    pub fn matches(&self, output: &[String]) -> bool {
        let rest: Vec<&String> = output
            .iter()
            .filter(|l| !(self.has_check && l.starts_with("checksum ")))
            .collect();
        rest.len() == self.expect.len() && rest.iter().zip(&self.expect).all(|(a, b)| *a == b)
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

/// The driver of `k` with `n` runs per benchmark.
pub fn driver(k: &Kernel, n: u32) -> Result<Driver, String> {
    let base = read("base.js")?;
    let src = read(k.file)?;
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

    /// The navier-stokes checksum after one frame in T0 equals node's: node v24.22.1 printed
    /// this line for `spike-c driver navier-stokes --n 1` on 2026-10-26 (the protocol's
    /// tier-differential reference for kernel 2; §5).
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
        assert_eq!(
            r.output,
            [
                "checksum 320.5614702136357 -372.6606391856326 -366.08057515126234",
                "NavierStokes 1"
            ]
        );
    }
}
