//! Unit-load interning cost (owner M6-P0 MODES6; MODES-CHECK-w23 MC-w23-11, M6-PLAN-D11). A
//! report test, `#[ignore]`: it prints numbers and asserts only the unit shape. The deciding
//! run is one orchestrator run on a quiet Mac (V11-A19); STR4 builds lazy intern only if the
//! cost is above 1 ms per 10k unit strings (V11-D11).
//!
//! ```text
//! EP9_HASH_SEED=1 cargo test --locked --release -p ep9_builtins --test intern_at_load \
//!     -- --ignored --nocapture --test-threads 1
//! ```
//!
//! T0 interns every unit string at load (`LoadedUnit::load`, the H19 "strings core"). The test
//! times `Cx::run_script` (load, then a top level that only declares functions) of three units
//! with the same shape, at 1k, 10k and 100k strings, median of 11 runs, each on a fresh
//! isolate (compile and isolate set-up are outside the timer):
//!
//! | Unit | What load does with the N slots |
//! |---|---|
//! | `strings` | N new unit strings, all new atoms (the cold load) |
//! | `strings again` | the same unit loaded a second time in the same isolate: N atom lookups that hit |
//! | `numbers` | N double constants, no unit string beyond the function names: the control |
//!
//! "strings - numbers" is the intern cost of N new strings; the V11-D11 reading is that cost
//! per 10k strings. ep9_vm tests cannot compile JS, so the test lives here.

use std::time::{Duration, Instant};

use ep9_bytecode::VerifiedUnit;
use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_vm::{Isolate, IsolateConfig};

/// Unit strings per function. Each function holds one array literal and is never called, so
/// the top level only declares the functions.
const PER_FN: usize = 1_000;
/// Runs per reading; the median is reported.
const RUNS: usize = 11;

/// The `i`-th unit string: an identifier-like text of 11-14 chars (not an array index).
fn text(i: usize) -> String {
    format!("intern_k{i}")
}

/// A script of `n / PER_FN` uncalled functions, each with an array literal of `PER_FN`
/// elements: the unit strings `text(i)`, or the doubles `i + 0.5` for the control.
fn source(n: usize, strings: bool) -> String {
    let mut s = String::with_capacity(n * 16);
    for f in 0..n / PER_FN {
        s.push_str(&format!("function f{f}() {{ return ["));
        for k in 0..PER_FN {
            let i = f * PER_FN + k;
            if strings {
                s.push_str(&format!("\"{}\",", text(i)));
            } else {
                s.push_str(&format!("{}.5,", i));
            }
        }
        s.push_str("]; }\n");
    }
    s
}

fn compile(src: &str) -> VerifiedUnit {
    ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("the unit compiles")
}

fn isolate() -> Isolate {
    Isolate::new(IsolateConfig {
        builtins: Some(&ep9_builtins::TABLE),
        ..IsolateConfig::default()
    })
    .expect("an isolate")
}

/// The time of one `run_script` of `unit` on `iso`.
fn load(iso: &mut Isolate, unit: VerifiedUnit) -> Duration {
    iso.with_cx(|cx| {
        let t = Instant::now();
        cx.run_script(unit, "intern.js")?;
        Ok(t.elapsed())
    })
    .expect("the script runs")
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

#[test]
#[ignore = "report test: the orchestrator runs it on a quiet Mac (V11-A19)"]
fn unit_load_interning_cost_per_10k_strings() {
    println!("intern-at-load (MC-w23-11, V11-D11): median of {RUNS} runs, release build expected");
    println!(
        "| unit strings | cold load ms | load again ms | numbers ms | intern ms (cold - numbers) | per 10k strings ms |"
    );
    println!("|---|---|---|---|---|---|");
    for n in [1_000, 10_000, 100_000] {
        let strings = compile(&source(n, true));
        let numbers = compile(&source(n, false));
        // The unit holds the N texts (plus the function names); the control holds none of
        // them, so its load interns only the function names.
        assert!(
            strings.unit().strings.len() >= n,
            "{n}: too few unit strings"
        );
        assert!(
            strings
                .unit()
                .strings
                .iter()
                .filter(|s| matches!(s, ep9_bytecode::UnitString::Latin1(b) if b.starts_with(b"intern_k")))
                .count()
                == n,
            "{n}: the unit does not hold every text"
        );
        assert!(
            numbers.unit().strings.len() < n / 10,
            "{n}: the control has strings"
        );
        let (mut cold, mut again, mut control) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..RUNS {
            let mut iso = isolate();
            cold.push(load(&mut iso, strings.clone()));
            again.push(load(&mut iso, strings.clone()));
            drop(iso);
            let mut iso = isolate();
            control.push(load(&mut iso, numbers.clone()));
        }
        let (c, a, k) = (median(cold), median(again), median(control));
        let intern = ms(c) - ms(k);
        println!(
            "| {n} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
            ms(c),
            ms(a),
            ms(k),
            intern,
            intern * 10_000.0 / n as f64
        );
    }
}
