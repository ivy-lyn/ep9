//! The tier differential of the protocol (C3; plan v10 M5-P3 SPIKEC-7, WI-SC6): T0 and Spike C
//! (config C) run the same JS, and the outputs must be equal byte for byte. Every guard of
//! each compiled function is forced at least once (a forced exit deopts into T0, which must
//! still print the same), and the method fails if a guard was never forced.
//!
//! ```text
//!  spike-c diff [--seeds A..B] [--no-kernels] [--show N]      (gate.sh diff: seeds 1..200;
//!                                                             --show: a program, its graph)
//!
//!  the gate kernels, n = 1:  crypto (am3), navier-stokes (lin_solve)
//!    node runs the driver  ──> its lines == the T0 lines        (node on PATH, else SKIP)
//!    config C, no forcing  ──> == T0, 0 exits (tier::unforced_ok)
//!    forcing (below)       ──> == T0 in every run, every guard forced
//!  the seeded corpus, seed s in A..=B:  program(s) = defs, warm, main, the function's name
//!    lint (no GC-timing output: no WeakRef, FinalizationRegistry or gc)
//!    T0:        defs ; warm ; main                      -> output A
//!    config C:  defs ; warm ; compile + install ; main  -> output B == A   (natural exits counted)
//!  forcing (each compiled function):
//!    1  every guard armed from the seed (countdown 1-8, emit_a64::ForceTable::seeded)
//!    2  each guard not forced yet armed at 1 (its first pass exits), again while one more is
//!       forced; every run == T0, and the VM counted at least the forced exit of each guard
//!    3  a guard never forced fails the method (it was never reached: the program does not
//!       test it)
//! ```
//!
//! **No GC-timing compare (MC-w17-10, V10-D34).** A JIT frame and a T0 frame differ, and the
//! GC scans native stacks conservatively (H10), so when a WeakRef target dies or a
//! FinalizationRegistry callback runs may differ between the two runs with no bug. The
//! generator never writes such code, and [`lint`] refuses a program (or a kernel driver) that
//! names `WeakRef`, `FinalizationRegistry` or the harness's `gc` anywhere, strings included
//! (fail closed). No stack scrub is done: that is the A3 differential's choice.
//!
//! **The corpus** ([`program`]). Each seed picks a family (`seed - 1` modulo the family
//! count, so every family gets its share of 1..200) and, from a splitmix64 stream of the seed,
//! the operators, constants, arrays and edge inputs. Each program has one function the
//! builder accepts (a refusal fails the method: the generator must stay inside the compiled
//! subset), a warm-up of 20 calls with plain inputs (the feedback the facts come from), then a
//! `main` that first calls it with plain inputs (so each guard is reached) and then with edge
//! inputs. The edge inputs are the WI-SC6 list: int32 overflow, double and int mixes, holes,
//! out-of-bounds indices, shape changes and `valueOf` (a counter shows each call ran once, so
//! an exit placed after a side effect would show), plus a DICTIONARY array (the kind guard
//! fails closed, R17), strings, `-0`, NaN and a throw inside a call. Every call in `main` is in
//! its own `try`, so a throw (in both runs) does not cut the rest.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::process::{Command, Stdio};

use spike_b::harness::Config;

use crate::CompileReport;
use crate::emit_a64::{EmitOptions, ForceTable};
use crate::kernels::{self, Kernel};
use crate::tier::{self, FORCE_LEN, IntFacts};

/// The seeds of `gate.sh diff` (plan v10 §5.7: seeds 1..200, est.).
pub const SEEDS: std::ops::RangeInclusive<u64> = 1..=200;

/// The gate kernels the differential runs at n = 1 (protocol §2: the two Role::Gate kernels).
pub const KERNELS: [&str; 2] = ["crypto", "navier-stokes"];

/// At most this many "arm each unforced guard at 1" rounds per function (each round forces at
/// least one more guard, or the loop ends).
const MAX_ROUNDS: usize = 64;

// ---- the lint (MC-w17-10) ------------------------------------------------------------------

/// The names whose use makes an output depend on GC timing: a WeakRef's target, a
/// FinalizationRegistry's callbacks, and the harness's `gc()` (a collection on request).
pub const GC_TIMING_NAMES: [&str; 3] = ["WeakRef", "FinalizationRegistry", "gc"];

/// Refuses `src` if it names any of [`GC_TIMING_NAMES`] as a whole word, anywhere (in a
/// string or a comment too: `globalThis["gc"]` reaches it, so the lint fails closed rather
/// than parse). Ok, or the names it found.
pub fn lint(src: &str) -> Result<(), String> {
    let mut found = BTreeSet::new();
    let b = src.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    let mut i = 0;
    while i < b.len() {
        if word(b[i]) && (i == 0 || !word(b[i - 1])) {
            let start = i;
            while i < b.len() && word(b[i]) {
                i += 1;
            }
            let w = &src[start..i];
            if GC_TIMING_NAMES.contains(&w) {
                found.insert(w);
            }
        } else {
            i += 1;
        }
    }
    if found.is_empty() {
        Ok(())
    } else {
        let names: Vec<&str> = found.into_iter().collect();
        Err(format!(
            "GC-timing names in the program: {} (MC-w17-10: the tier diff compares no GC timing)",
            names.join(", ")
        ))
    }
}

// ---- the generator -------------------------------------------------------------------------

/// splitmix64: the generator's stream.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x5350_494B_4543_3743)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// A number in `lo..=hi`.
    fn int(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo + 1) as u64) as i64
    }

    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())]
    }

    /// `k` distinct items of `xs` (all of them if `k >= xs.len()`), in `xs` order.
    fn some<T: Copy>(&mut self, xs: &[T], k: usize) -> Vec<T> {
        let mut idx: Vec<usize> = (0..xs.len()).collect();
        for i in (1..idx.len()).rev() {
            let j = self.below(i + 1);
            idx.swap(i, j);
        }
        let mut chosen: Vec<usize> = idx.into_iter().take(k).collect();
        chosen.sort_unstable();
        chosen.into_iter().map(|i| xs[i]).collect()
    }

    /// A JS array literal of `n` small int32s in `lo..=hi`.
    fn ints(&mut self, n: usize, lo: i64, hi: i64) -> String {
        let v: Vec<String> = (0..n).map(|_| self.int(lo, hi).to_string()).collect();
        format!("[{}]", v.join(", "))
    }

    /// [`Rng::ints`] with `x` at one place (`n >= 2`) and some other value at another.
    fn ints_with(&mut self, n: usize, lo: i64, hi: i64, x: i64) -> String {
        let mut v: Vec<i64> = (0..n).map(|_| self.int(lo, hi)).collect();
        let at = self.below(n);
        v[at] = x;
        let other = (at + 1) % n;
        if v[other] == x {
            v[other] = if x == hi { lo } else { x + 1 };
        }
        let v: Vec<String> = v.iter().map(i64::to_string).collect();
        format!("[{}]", v.join(", "))
    }

    /// A JS double literal that is no int32 (`k.25`, `k.5` or `k.75`, maybe negative).
    fn dbl(&mut self) -> String {
        let frac = self.pick(&["25", "5", "75", "125"]);
        let k = self.int(-9, 9);
        format!("{k}.{frac}")
    }

    /// A JS array literal of `n` doubles (no int32 among them).
    fn dbls(&mut self, n: usize) -> String {
        let v: Vec<String> = (0..n).map(|_| self.dbl()).collect();
        format!("[{}]", v.join(", "))
    }
}

/// One program of the corpus.
#[derive(Clone, Debug)]
pub struct Program {
    pub seed: u64,
    pub family: &'static str,
    /// The function Spike C compiles (a global function, or an inner closure by its id).
    pub name: &'static str,
    pub defs: String,
    pub warm: String,
    pub main: String,
}

impl Program {
    /// The whole program as one text (the lint input; `spike-c diff --show`).
    pub fn text(&self) -> String {
        format!(
            "// seed {} ({}), compiled: {}\n// ---- defs\n{}\n// ---- warm\n{}\n// ---- main\n{}\n",
            self.seed, self.family, self.name, self.defs, self.warm, self.main
        )
    }
}

/// The helpers every program's `defs` starts with: `V(x)`, an object whose `valueOf` counts
/// its calls and returns `x`; `D(a)`, `a` moved to DICTIONARY elements (an index far past its
/// capacity, elements_kind.rs `ARRAY_GAP`), with its first elements kept; `O1`, `O2`, two
/// objects a program can store and find again by identity; `K(x)`, `x` kept in the list `R`
/// of input objects (with the objects among its first 256 elements); `S(a)`, an array shown
/// element by element (Codex M5-P3 r1 spikec7 2, r2 spk 2): its length, a hole as `hole`,
/// `-0` as itself, a primitive with its type, an object by strict identity: `self`, `r<j>`
/// (`R[j]`; `O1` is `r0`, `O2` is `r1`) or `n<j>` (the j-th other object `S` met in `a`).
/// Up to length 256 it shows every index; a longer array shows each own key as `key=value`
/// (every present index, however far). Print's ToString would map -0 to 0, a hole, null and
/// undefined to "", and every object alike; content labels would merge objects alike.
const PRELUDE: &str = "\
var calls = 0;\n\
function V(x) { return { valueOf: function () { calls = calls + 1; return x; } }; }\n\
function D(a) { a[100000] = 0; return a; }\n\
var O1 = { id: 1 }, O2 = { id: 2 };\n\
var R = [O1, O2];\n\
function isObj(x) { return typeof x === 'object' && x !== null; }\n\
function K(x) { if (isObj(x)) { if (R.indexOf(x) < 0) R.push(x); if (x.length <= 256) for (var i = 0; i < x.length; i++) { var e = x[i]; if (isObj(e) && R.indexOf(e) < 0) R.push(e); } } return x; }\n\
function S(a) { if (!isObj(a)) return typeof a + ':' + String(a); var o = ['len ' + a.length], n = []; function show(x) { if (x === 0 && 1 / x < 0) return '-0'; if (!isObj(x)) return typeof x + ':' + String(x); if (x === a) return 'self'; var j = R.indexOf(x); if (j >= 0) return 'r' + j; j = n.indexOf(x); if (j < 0) { j = n.length; n.push(x); } return 'n' + j; } if (a.length <= 256) { for (var i = 0; i < a.length; i++) o.push(i in a ? show(a[i]) : 'hole'); } else { var ks = Object.keys(a); for (var k = 0; k < ks.length; k++) o.push(ks[k] + '=' + show(a[ks[k]])); } return o.join(' '); }\n";

/// `expr`, a call that returns an array, shown by `S` (PRELUDE), with each argument kept
/// by `K` ([`retained`]); a trailing valueOf count stays outside.
fn shown(expr: &str) -> String {
    match expr.strip_suffix(" + ' ' + calls") {
        Some(call) => format!("S({}) + ' ' + calls", retained(call)),
        None => format!("S({})", retained(expr)),
    }
}

/// The call `name(a, b, ..)` as `name(K(a), K(b), ..)`, so `S` names its input objects by
/// identity (Codex M5-P3 r2 spk 2). Any other form (no plain name, or more after the call's
/// closing parenthesis) stays as it is.
fn retained(call: &str) -> String {
    let Some(open) = call.find('(') else {
        return call.to_string();
    };
    let name = &call[..open];
    let plain = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let Some(inner) = call[open + 1..].strip_suffix(')') else {
        return call.to_string();
    };
    if !plain {
        return call.to_string();
    }
    let (mut depth, mut start, mut quote, mut args) = (0i32, 0, None, Vec::new());
    for (i, c) in inner.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth < 0 {
                    return call.to_string();
                }
            }
            ',' if depth == 0 => {
                args.push(inner[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 || quote.is_some() {
        return call.to_string();
    }
    args.push(inner[start..].trim());
    let args: Vec<String> = args
        .iter()
        .map(|a| match a.parse::<f64>() {
            Ok(_) => a.to_string(),
            Err(_) => format!("K({a})"),
        })
        .collect();
    format!("{name}({})", args.join(", "))
}

/// A line of `main`: `expr` printed, a throw printed by its name.
fn line(expr: &str) -> String {
    format!("try {{ print({expr}); }} catch (e) {{ print('throw ' + e.name); }}\n")
}

/// The families of the corpus, in seed order (seed `s` is family `(s - 1) % len`).
pub const FAMILIES: [&str; 12] = [
    "int-sum", "int-ovf", "dbl", "store", "copy", "named", "branch", "closure", "call", "nested",
    "div-eq", "lin",
];

/// The program of `seed` (deterministic).
pub fn program(seed: u64) -> Program {
    let mut r = Rng::new(seed);
    let family = FAMILIES[((seed.max(1) - 1) % FAMILIES.len() as u64) as usize];
    let mut defs = String::from(PRELUDE);
    let mut main = String::new();
    let warm;
    let name;
    match family {
        "int-sum" => {
            name = "f";
            let (op1, op2) = (r.pick(&["+", "-", "^"]), r.pick(&["*", "+", "&", "<<"]));
            let c0 = r.int(-5, 5);
            defs += &format!(
                "function f(a, n, k) {{ var s = {c0}; for (var i = 0; i < n; i++) {{ s = (s {op1} a[i] {op2} k) | 0; }} return s; }}\n"
            );
            let k = r.int(1, 5);
            let w = r.ints(6, -50, 50);
            warm = format!("var W = {w}; for (var q = 0; q < 20; q++) f(W, W.length, {k});");
            for _ in 0..3 {
                let n = r.int(3, 8) as usize;
                main += &line(&format!(
                    "f({}, {n}, {})",
                    r.ints(n, -1000, 1000),
                    r.int(0, 9)
                ));
            }
            let edges = [
                format!("f([2147483647, 2147483647, 1073741824], 3, {k})"),
                format!("f([1, {}, 3], 3, {k})", r.dbl()),
                format!("f([1, , 3], 3, {k})"),
                format!("f([1, 2, 3], 5, {k})"),
                format!("f([1, V(4), 3], 3, {k}) + ' ' + calls"),
                format!("f([1, '7', 3], 3, {k})"),
                format!("f([1, 2, 3], 3, {})", r.dbl()),
                format!("f([1, 2, 3], 3, V({k})) + ' ' + calls"),
                format!("f(D([1, 2, 3]), 3, {k})"),
                format!("f({{ 0: 1, 1: 2, length: 2 }}, 2, {k})"),
                format!("f([-0, 1], 2, {k})"),
                format!("f([1, 2, 3], 40, {k})"),
            ];
            for e in r.some(&edges.each_ref(), 6) {
                main += &line(e);
            }
        }
        "int-ovf" => {
            name = "f";
            let body = r.pick(&[
                "s = s + a[i];",
                "s = s - a[i] * 3;",
                "s = s + a[i] + i;",
                "s = s + (a[i] << 4);",
            ]);
            defs += &format!(
                "function f(a, n) {{ var s = 0; for (var i = 0; i < n; i++) {{ {body} }} return s; }}\n"
            );
            let w = r.ints(5, -100, 100);
            warm = format!("var W = {w}; for (var q = 0; q < 20; q++) f(W, W.length);");
            for _ in 0..3 {
                let n = r.int(2, 7) as usize;
                main += &line(&format!("f({}, {n})", r.ints(n, -100000, 100000)));
            }
            let big = r.pick(&["2147483647", "1073741824", "-2147483648", "2147483000"]);
            let edges = [
                format!("f([{big}, {big}, {big}], 3)"),
                format!("f([{big}, -1, {big}, 5], 4)"),
                format!("f([1, {}, 3], 3)", r.dbl()),
                "f([1, , 3], 3)".to_string(),
                "f([1, 2], 4)".to_string(),
                "f([V(2147483647), 1], 2) + ' ' + calls".to_string(),
                "f(D([5, 6, 7]), 3)".to_string(),
                "f(['a', 1], 2)".to_string(),
                "f([NaN, 1], 2)".to_string(),
            ];
            for e in r.some(&edges.each_ref(), 6) {
                main += &line(e);
            }
        }
        "dbl" => {
            name = "f";
            let op = r.pick(&["+", "-", "/", "*"]);
            defs += &format!(
                "function f(a, b, n, x) {{ var s = 0; for (var i = 0; i < n; i++) {{ s = s + a[i] * x {op} b[i]; }} return s; }}\n"
            );
            let (wa, wb, x) = (r.dbls(4), r.dbls(4), r.dbl());
            warm = format!(
                "var WA = {wa}, WB = {wb}; for (var q = 0; q < 20; q++) f(WA, WB, 4, {x});"
            );
            for _ in 0..3 {
                let n = r.int(2, 6) as usize;
                main += &line(&format!(
                    "f({}, {}, {n}, {})",
                    r.dbls(n),
                    r.dbls(n),
                    r.dbl()
                ));
            }
            let edges = [
                format!("f([1, 2, 3], [4, 5, 6], 3, {x})"),
                format!("f([NaN, 1.5], [1.5, 2.5], 2, {x})"),
                "1 / f([-0, 0.5], [0.5, 0.5], 1, -0)".to_string(),
                "f([1e308, 1e308], [0.5, 0.5], 2, 10)".to_string(),
                format!("f([1.5, , 2.5], [1.5, 1.5, 1.5], 3, {x})"),
                format!("f([1.5, 2.5], [1.5, 2.5], 4, {x})"),
                format!("f([1.5, 'x'], [1.5, 2.5], 2, {x})"),
                format!("f([1.5, V(2.5)], [1.5, 2.5], 2, {x}) + ' ' + calls"),
                "f([1.5, 2.5], [1.5, 2.5], 2, '3')".to_string(),
                format!("f(D([1.5, 2.5]), [1.5, 2.5], 2, {x})"),
                "f([1.5, 2.5], [1.5, 2.5], 2, undefined)".to_string(),
                format!("f([0.5, 1, 2147483647], [3, 0.5, 2], 3, {x})"),
            ];
            for e in r.some(&edges.each_ref(), 6) {
                main += &line(e);
            }
        }
        "store" => {
            name = "f";
            // On doubles, no `| 0` or `^`: a double that is not an int32 exits at its
            // TruncateInt32 (the spike truncates only int32 Values), on every call.
            let dbl = r.below(2) == 0;
            let expr = if dbl {
                r.pick(&["b[i] * v", "b[i] + i", "b[i] - v"])
            } else {
                r.pick(&[
                    "(b[i] + v) | 0",
                    "b[i] * v",
                    "b[i] + i",
                    "(b[i] ^ v) + 1",
                    "b[i] - v",
                ])
            };
            defs += &format!(
                "function f(a, b, n, v) {{ for (var i = 0; i < n; i++) {{ a[i] = {expr}; }} return a; }}\n"
            );
            let (wb, v) = if dbl {
                (r.dbls(4), r.dbl())
            } else {
                (r.ints(4, -100, 100), r.int(1, 9).to_string())
            };
            warm = format!(
                "var WB = {wb}; for (var q = 0; q < 20; q++) f({}, WB, 4, {v});",
                if dbl {
                    "[0.5, 0.5, 0.5, 0.5]"
                } else {
                    "[0, 0, 0, 0]"
                }
            );
            for _ in 0..3 {
                let n = r.int(2, 6) as usize;
                let b = if dbl {
                    r.dbls(n)
                } else {
                    r.ints(n, -1000, 1000)
                };
                let a = if dbl { r.dbls(n) } else { r.ints(n, -9, 9) };
                main += &line(&shown(&format!("f({a}, {b}, {n}, {v})")));
            }
            let d = r.dbl();
            let edges = [
                format!("f([0, 0, 0], [1, 2, 3], 3, {d})"),
                format!("f([0.5, 0.5, 0.5], [1, 2, 3], 3, {v})"),
                format!("f([0, 0], [1, 2, 3], 3, {v})"),
                format!("f([0, , 0], [1, 2, 3], 3, {v})"),
                format!("f(D([0, 0, 0]), [1, 2, 3], 3, {v})"),
                format!("f(Object.freeze([0, 0, 0]), [1, 2, 3], 3, {v})"),
                format!("f([0, 0, 0], [1, , 3], 3, {v})"),
                format!("f([0, 0, 0], [1, 'x', 3], 3, {v})"),
                "f([0, 0, 0], [2147483647, 1, 1], 3, 2147483647)".to_string(),
                format!("f(['s', 0, 0], [1, 2, 3], 3, {v})"),
                format!("f([0, 0, 0], [1, 2, 3], 3, V({v})) + ' ' + calls"),
                "f([0, 0, 0], [1, 2, 3], 3, NaN)".to_string(),
            ];
            for e in r.some(&edges.each_ref(), 6) {
                main += &line(&shown(e));
            }
        }
        "copy" => {
            name = "f";
            let o = r.int(0, 2);
            defs += &format!(
                "function f(a, b, n) {{ for (var i = 0; i < n; i++) {{ a[i + {o}] = b[i]; }} return a; }}\n"
            );
            let dbl = r.below(2) == 0;
            let wb = if dbl { r.dbls(4) } else { r.ints(4, -50, 50) };
            warm = format!(
                "var WB = {wb}; for (var q = 0; q < 20; q++) f({}, WB, 4);",
                if dbl {
                    "[0.5, 0.5, 0.5, 0.5, 0.5, 0.5]"
                } else {
                    "[0, 0, 0, 0, 0, 0]"
                }
            );
            for _ in 0..3 {
                let n = r.int(2, 5) as usize;
                let b = if dbl {
                    r.dbls(n)
                } else {
                    r.ints(n, -1000, 1000)
                };
                let a = if dbl {
                    r.dbls(n + 2)
                } else {
                    r.ints(n + 2, -9, 9)
                };
                main += &line(&shown(&format!("f({a}, {b}, {n})")));
            }
            let edges = [
                "f([0, 0, 0, 0, 0], ['a', 'b', 'c'], 3)".to_string(),
                "f([0.5, 0.5, 0.5, 0.5, 0.5], [1, -0, 3], 3)".to_string(),
                "f([0, 0, 0, 0, 0], [1.5, 2.5, 3.5], 3)".to_string(),
                "f([0, 0, 0, 0, 0], [1, , 3], 3)".to_string(),
                "f([0, 0, 0, 0, 0], [O1, null, undefined], 3)".to_string(),
                "f([0, 0], [O2, {}, 3], 3)".to_string(),
                "f(D([0, 0, 0, 0, 0]), [1, 2, 3], 3)".to_string(),
                "f([0, 0, 0, 0, 0], D([1, 2, 3]), 3)".to_string(),
                "f(['x', 0, 0, 0, 0], [1.5, 2, 3], 3)".to_string(),
                "f([0, 0, 0, 0, 0], [NaN, -0, 1], 3)".to_string(),
            ];
            for e in r.some(&edges.each_ref(), 6) {
                main += &line(&shown(e));
            }
        }
        "named" => {
            name = "f";
            let m = r.int(1, 4);
            defs += "function P(x, y) { this.x = x; this.y = y; }\n";
            defs += &format!(
                "function f(o, n) {{ var s = 0; for (var i = 0; i < n; i++) {{ s = (s + o.x * {m} + o.y * (i + 1)) | 0; }} return s; }}\n"
            );
            warm = format!(
                "var WP = new P({}, {}); for (var q = 0; q < 20; q++) f(WP, 4);",
                r.int(-9, 9),
                r.int(-9, 9)
            );
            for _ in 0..3 {
                main += &line(&format!(
                    "f(new P({}, {}), {})",
                    r.int(-500, 500),
                    r.int(-500, 500),
                    r.int(1, 8)
                ));
            }
            let edges = [
                "f({ x: 1, y: 2 }, 3)".to_string(),
                "f({ y: 2, x: 1 }, 3)".to_string(),
                "(function () { var p = new P(1, 2); p.z = 3; return f(p, 3); })()".to_string(),
                format!("f(new P({}, 2), 3)", r.dbl()),
                "f(new P(1, '2'), 3)".to_string(),
                "f({ get x() { calls = calls + 1; return 5; }, y: 1 }, 3) + ' ' + calls"
                    .to_string(),
                "f(null, 3)".to_string(),
                "f(new P(2147483647, 2147483647), 3)".to_string(),
                "f(new P(V(3), 1), 2) + ' ' + calls".to_string(),
                "(function () { var p = new P(1, 2); delete p.x; return f(p, 2); })()".to_string(),
            ];
            for e in r.some(&edges.each_ref(), 6) {
                main += &line(e);
            }
        }
        "branch" => {
            name = "f";
            let cmp = r.pick(&["<", ">", "<=", ">=", "==="]);
            defs += &format!(
                "function f(a, n, t) {{ var s = 0, c = 0; for (var i = 0; i < n; i++) {{ if (a[i] {cmp} t) {{ s = (s + a[i]) | 0; }} else {{ c = c + 1; }} }} return s * 1000 + c; }}\n"
            );
            warm = "var W = [1, 5, 3, 7, 3, 0]; for (var q = 0; q < 20; q++) f(W, W.length, 3);"
                .to_string();
            for _ in 0..3 {
                let n = r.int(4, 9) as usize;
                // `t` is in the array, so both arms of the `if` run (each guard is reached).
                let t = r.int(-3, 3);
                main += &line(&format!("f({}, {n}, {t})", r.ints_with(n, -5, 5, t)));
            }
            let edges = [
                format!("f([1, {}, 3], 3, 2)", r.dbl()),
                "f([1, 2, 3], 3, 2.5)".to_string(),
                "f([1, , 3], 3, 2)".to_string(),
                "f([1, 2, 3], 5, 2)".to_string(),
                "f([2147483647, 2147483647, 5], 3, 0)".to_string(),
                "f([1, 'b', 3], 3, 2)".to_string(),
                "f([1, V(9), 3], 3, 2) + ' ' + calls".to_string(),
                "f(D([1, 2, 3]), 3, 2)".to_string(),
                "f([3, 3, 3], 3, 3)".to_string(),
                "f([1, 2, 3], 3, NaN)".to_string(),
            ];
            for e in r.some(&edges.each_ref(), 6) {
                main += &line(e);
            }
        }
        "closure" => {
            name = "g";
            defs += "function mk(w, h) { return function g(a, n) { var s = 0; for (var i = 0; i < n; i++) { s = (s + a[i] * w + h) | 0; } return s; }; }\n";
            defs += &format!(
                "var g1 = mk({}, {}), g2 = mk({}, {}), g3 = mk({}, 1), g4 = mk('2', 1), g5 = mk(2147483647, 0);\n",
                r.int(-9, 9),
                r.int(-9, 9),
                r.int(-9, 9),
                r.int(-9, 9),
                r.dbl()
            );
            warm = "var W = [1, 2, 3, 4]; for (var q = 0; q < 20; q++) g1(W, 4);".to_string();
            for f in ["g1", "g2", "g1"] {
                let n = r.int(2, 7) as usize;
                main += &line(&format!("{f}({}, {n})", r.ints(n, -1000, 1000)));
            }
            let edges = [
                "g3([1, 2, 3], 3)",
                "g4([1, 2, 3], 3)",
                "g5([1, 2, 3], 3)",
                "g1([1, 2.5, 3], 3)",
                "g2([1, , 3], 3)",
                "g1([1, 2], 3)",
                "g2([V(5), 1], 2) + ' ' + calls",
                "g1(D([4, 5, 6]), 3)",
            ];
            for e in r.some(&edges, 5) {
                main += &line(e);
            }
        }
        "call" => {
            name = "g";
            let m = r.int(2, 5);
            defs += "var hcalls = 0;\n";
            defs += &format!(
                "function mk(m) {{ function h(x) {{ hcalls = hcalls + 1; if (x === 13) throw new RangeError('13'); return x * m + 1; }} return function g(a, n) {{ var s = 0; for (var i = 0; i < n; i++) {{ s = (s + h(a[i])) | 0; }} return s; }}; }}\nvar g1 = mk({m}), g2 = mk(0.5);\n"
            );
            warm = "var W = [1, 2, 3, 4]; for (var q = 0; q < 20; q++) g1(W, 4); hcalls = 0;"
                .to_string();
            for _ in 0..3 {
                let n = r.int(2, 6) as usize;
                main += &line(&format!("g1({}, {n}) + ' ' + hcalls", r.ints(n, -100, 100)));
            }
            let edges = [
                "g2([1, 2, 3], 3) + ' ' + hcalls",
                "g1([1, 2.5, 3], 3) + ' ' + hcalls",
                "g1([1, 13, 3], 3) + ' ' + hcalls",
                "g1([1, , 3], 3) + ' ' + hcalls",
                "g1([2147483647, 2147483647], 2) + ' ' + hcalls",
                "g1([1, 2], 4) + ' ' + hcalls",
                "g1([1, 'x'], 2) + ' ' + hcalls",
                "g1(D([1, 2]), 2) + ' ' + hcalls",
            ];
            for e in r.some(&edges, 5) {
                main += &line(e);
            }
            main += "print('h calls ' + hcalls);\n";
        }
        "nested" => {
            name = "f";
            let m = r.pick(&["(i + 1)", "j", "(i ^ j)", "2"]);
            defs += &format!(
                "function f(a, w, h) {{ var s = 0; for (var j = 0; j < h; j++) {{ var row = j * w; for (var i = 0; i < w; i++) {{ s = (s + a[row + i] * {m}) | 0; }} }} return s; }}\n"
            );
            warm = format!(
                "var W = {}; for (var q = 0; q < 20; q++) f(W, 3, 3);",
                r.ints(9, -20, 20)
            );
            for _ in 0..3 {
                let (w, h) = (r.int(1, 4) as usize, r.int(1, 4) as usize);
                main += &line(&format!("f({}, {w}, {h})", r.ints(w * h, -1000, 1000)));
            }
            let edges = [
                "f([1, 2, 3, 4], 2, 3)".to_string(),
                format!("f([1, 2, {}, 4], 2, 2)", r.dbl()),
                "f([1, 2, , 4], 2, 2)".to_string(),
                "f([2147483647, 2147483647, 2147483647, 2147483647], 2, 2)".to_string(),
                "f([1, V(3), 3, 4], 2, 2) + ' ' + calls".to_string(),
                "f(D([1, 2, 3, 4]), 2, 2)".to_string(),
                "f([1, 2, 3, 4], 0, 3)".to_string(),
                "f([1, 2, 3, 4], 2, 2.5)".to_string(),
            ];
            for e in r.some(&edges.each_ref(), 5) {
                main += &line(e);
            }
        }
        "div-eq" => {
            name = "f";
            let d = r.int(2, 7);
            defs += "function f(a, n, k, d) { var c = 0, s = 0; for (var i = 0; i < n; i++) { if (a[i] === k) { c = c + 1; } s = s + a[i] / d; } return s + c; }\n";
            warm = "var W = [1, 2, 3, 2]; for (var q = 0; q < 20; q++) f(W, 4, 2, 3);".to_string();
            for _ in 0..3 {
                let n = r.int(2, 7) as usize;
                let k = r.int(-4, 4);
                main += &line(&format!("f({}, {n}, {k}, {d})", r.ints_with(n, -4, 4, k)));
            }
            let edges = [
                format!("f([1, {}, 3], 3, 1, {d})", r.dbl()),
                "f([1, 2, 3], 3, 1, 0)".to_string(),
                "f([1, 2, 3], 3, 1, V(2)) + ' ' + calls".to_string(),
                format!("f([1, 2, 3], 3, 2.5, {d})"),
                format!("f([1, , 3], 3, 1, {d})"),
                format!("f([1, 2], 3, 1, {d})"),
                "1 / f([0, -0, 0], 3, 0, -1)".to_string(),
                format!("f([1, '1', 1], 3, 1, {d})"),
                format!("f([1, V(2), 1], 3, 1, {d}) + ' ' + calls"),
                format!("f(D([1, 2, 3]), 3, 1, {d})"),
                format!("f([NaN, 1, 2], 3, NaN, {d})"),
            ];
            for e in r.some(&edges.each_ref(), 6) {
                main += &line(e);
            }
        }
        _ => {
            // "lin": a lin_solve-like stencil over doubles, with its sizes in its context.
            name = "lin";
            let (w, h) = (r.int(2, 4), r.int(2, 3));
            defs += "function mk(width, height) { var rowSize = width + 2; return function lin(x, x0, a, c) { var invC = 1 / c; for (var j = 1; j <= height; j++) { var cur = j * rowSize + 1; var last = x[cur - 1]; for (var i = 1; i <= width; i++) { last = x[cur] = (x0[cur] + a * (last + x[cur + 1] + x[cur - rowSize] + x[cur + rowSize])) * invC; cur++; } } return x; }; }\n";
            defs += &format!("var L = mk({w}, {h}), L2 = mk({}, {});\n", w + 1, h);
            defs += "function field(w, h, k) { var a = new Array((w + 2) * (h + 2)); for (var i = 0; i < a.length; i++) a[i] = (i % 7) * k + 0.5; return a; }\n";
            let (a, c) = (r.dbl(), r.int(2, 6));
            warm = format!(
                "for (var q = 0; q < 20; q++) L(field({w}, {h}, 0.25), field({w}, {h}, 0.5), {a}, {c});"
            );
            for _ in 0..3 {
                main += &line(&shown(&format!(
                    "L(field({w}, {h}, {}), field({w}, {h}, {}), {}, {})",
                    r.dbl(),
                    r.dbl(),
                    r.dbl(),
                    r.int(1, 9)
                )));
            }
            let edges = [
                format!(
                    "L(field({}, {h}, 0.5), field({}, {h}, 0.25), {a}, {c})",
                    w + 1,
                    w + 1
                ),
                format!(
                    "L2(field({}, {h}, 0.5), field({}, {h}, 0.25), {a}, {c})",
                    w + 1,
                    w + 1
                ),
                format!("L(field({w}, {h}, 1), field({w}, {h}, 1), 2, 1)"),
                format!("L(field({w}, {h}, 0.5), field({w}, {h}, 0.5), 0, 1)"),
                format!(
                    "(function () {{ var x = field({w}, {h}, 0.5); x[{}] = 'x'; return L(x, field({w}, {h}, 0.5), {a}, {c}); }})()",
                    w + 3
                ),
                format!(
                    "(function () {{ var x0 = field({w}, {h}, 0.5); delete x0[{}]; return L(field({w}, {h}, 0.5), x0, {a}, {c}); }})()",
                    w + 4
                ),
                format!("L(field({w}, {h}, 0.5), [1, 2], {a}, {c})"),
                format!("L(D(field({w}, {h}, 0.5)), field({w}, {h}, 0.5), {a}, {c})"),
                format!("L(field({w}, {h}, 0.5), field({w}, {h}, 0.5), V(2), {c}) + ' ' + calls"),
                format!("L(field({w}, {h}, 0.5), field({w}, {h}, 0.5), {a}, 0)"),
            ];
            for e in r.some(&edges.each_ref(), 6) {
                main += &line(&shown(e));
            }
        }
    }
    main += "print('valueOf calls ' + calls);\n";
    Program {
        seed,
        family,
        name,
        defs,
        warm,
        main,
    }
}

// ---- the runs ------------------------------------------------------------------------------

/// What forcing found for one compiled function.
#[derive(Clone, Debug, Default)]
pub struct Forcing {
    /// Every guard of the exit table.
    pub guards: BTreeSet<u32>,
    /// The guards whose forced exit ran (and the VM counted it).
    pub forced: BTreeSet<u32>,
    /// The compiled runs made with a force table.
    pub runs: usize,
    /// Lines that fail the method (an output that differs, a forced exit not counted).
    pub bad: Vec<String>,
}

impl Forcing {
    /// The guards never forced.
    pub fn never(&self) -> Vec<u32> {
        self.guards.difference(&self.forced).copied().collect()
    }
}

/// The exits counted per guard (`exits` in record order, as `ScriptRun::exits`).
pub fn per_guard(report: &CompileReport, exits: &[u64]) -> BTreeMap<u32, u64> {
    let mut m = BTreeMap::new();
    for (rec, &c) in report.emit.meta.exits.iter().zip(exits) {
        *m.entry(rec.guard).or_insert(0) += c;
    }
    m
}

/// The op of each guard of a compile, from its graph text (`nK = Op(...) ... guard gN`).
pub fn guard_ops(report: &CompileReport) -> BTreeMap<u32, String> {
    let mut m = BTreeMap::new();
    for l in report.graph.lines() {
        let Some(at) = l.find(" guard g") else {
            continue;
        };
        let id: String = l[at + 8..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let Ok(id) = id.parse::<u32>() else { continue };
        let op = l
            .split_once(" = ")
            .map(|(_, r)| r)
            .unwrap_or("")
            .split(['(', ' ', '{'])
            .next()
            .unwrap_or("")
            .to_string();
        m.insert(id, op);
    }
    m
}

/// Forcing of one compiled function (module doc, steps 1-3): `run` makes one compiled run
/// with the given force table and returns whether its output equals T0's, its compile report
/// and its exits (record order).
pub fn force_all<F>(seed: u64, guards: BTreeSet<u32>, mut run: F) -> Result<Forcing, String>
where
    F: FnMut(&ForceTable) -> Result<(bool, CompileReport, Vec<u64>), String>,
{
    let mut f = Forcing {
        guards,
        ..Default::default()
    };
    let mut table = Some(ForceTable::seeded(seed, FORCE_LEN));
    let mut rounds = 0;
    while let Some(t) = table.take() {
        let armed = t.snapshot();
        let (equal, report, exits) = run(&t)?;
        f.runs += 1;
        if !equal {
            f.bad
                .push(format!("forced run {}: output differs from T0", f.runs));
        }
        let counted = per_guard(&report, &exits);
        let mut more = 0;
        for &g in &f.guards {
            let on = armed.get(g as usize).is_some_and(|&c| c != 0);
            if on && t.get(g) == 0 {
                if counted.get(&g).copied().unwrap_or(0) == 0 {
                    f.bad.push(format!(
                        "forced run {}: g{g} forced, but the VM counted no exit",
                        f.runs
                    ));
                }
                if f.forced.insert(g) {
                    more += 1;
                }
            }
        }
        let never = f.never();
        rounds += 1;
        if !never.is_empty() && (more > 0 || rounds == 1) && rounds <= MAX_ROUNDS {
            let next = ForceTable::new(FORCE_LEN);
            for g in never {
                next.arm(g, 1);
            }
            table = Some(next);
        }
    }
    Ok(f)
}

/// The result of one corpus program.
#[derive(Clone, Debug)]
pub struct ProgramRun {
    pub program: Program,
    /// The compile report (None if the builder refused: a failure).
    pub report: Option<CompileReport>,
    /// The unforced compiled output equals T0's.
    pub equal: bool,
    /// The natural exits of the unforced run, per guard.
    pub natural: BTreeMap<u32, u64>,
    pub forcing: Forcing,
    /// Why the program fails, if it does.
    pub error: Option<String>,
}

impl ProgramRun {
    pub fn ok(&self) -> bool {
        self.error.is_none()
            && self.report.is_some()
            && self.equal
            && self.forcing.bad.is_empty()
            && self.forcing.never().is_empty()
            && !self.forcing.guards.is_empty()
    }

    /// One report line.
    pub fn line(&self) -> String {
        let p = &self.program;
        if let Some(e) = &self.error {
            return format!("seed {} ({}): FAIL: {e}", p.seed, p.family);
        }
        let natural: Vec<String> = self
            .natural
            .iter()
            .filter(|(_, c)| **c > 0)
            .map(|(g, c)| format!("g{g}:{c}"))
            .collect();
        let never: Vec<String> = self
            .forcing
            .never()
            .iter()
            .map(|g| format!("g{g}"))
            .collect();
        format!(
            "seed {} ({}, {}): {}; {} guards, natural exits {}; forced {} of {} in {} runs{}{}",
            p.seed,
            p.family,
            p.name,
            if self.equal {
                "output equal to T0"
            } else {
                "OUTPUT DIFFERS"
            },
            self.forcing.guards.len(),
            if natural.is_empty() {
                "none".to_string()
            } else {
                natural.join(" ")
            },
            self.forcing.forced.len(),
            self.forcing.guards.len(),
            self.forcing.runs,
            if never.is_empty() {
                String::new()
            } else {
                format!("; NEVER FORCED {}", never.join(" "))
            },
            if self.forcing.bad.is_empty() {
                String::new()
            } else {
                format!("; {}", self.forcing.bad.join("; "))
            },
        )
    }
}

/// Runs the program of `seed` (module doc).
pub fn run_program(seed: u64, config: &Config) -> ProgramRun {
    let program = program(seed);
    let mut out = ProgramRun {
        program: program.clone(),
        report: None,
        equal: false,
        natural: BTreeMap::new(),
        forcing: Forcing::default(),
        error: None,
    };
    if let Err(e) = lint(&program.text()) {
        out.error = Some(e);
        return out;
    }
    let p = &program;
    let t0 = match tier::t0_script(config, &p.defs, &p.warm, &p.main) {
        Ok(t0) => t0,
        Err(e) => {
            out.error = Some(e);
            return out;
        }
    };
    if t0.result.is_err() {
        out.error = Some(format!("T0 main ended with {:?}", t0.result));
        return out;
    }
    let compiled = |opts: &EmitOptions<'_>| {
        tier::compiled_script(
            config,
            &p.defs,
            &p.warm,
            &p.main,
            p.name,
            IntFacts::Live,
            opts,
        )
    };
    let plain = match compiled(&EmitOptions::default()) {
        Ok(c) => c,
        Err(e) => {
            out.error = Some(e);
            return out;
        }
    };
    out.equal = plain.output == t0;
    if !out.equal {
        out.error = Some(format!(
            "output differs: compiled {:?} {:?} vs T0 {:?} {:?}",
            plain.output.result, plain.output.output, t0.result, t0.output
        ));
    }
    out.natural = per_guard(&plain.report, &plain.exits);
    let guards: BTreeSet<u32> = plain
        .report
        .emit
        .meta
        .exits
        .iter()
        .map(|e| e.guard)
        .collect();
    let forcing = force_all(seed, guards, |t| {
        let opts = EmitOptions {
            force: Some(t),
            ..Default::default()
        };
        let c = compiled(&opts)?;
        Ok((c.output == t0, c.report, c.exits))
    });
    match forcing {
        Ok(f) => out.forcing = f,
        Err(e) => out.error = Some(e),
    }
    out.report = Some(plain.report);
    out
}

/// node's output of `text` (stdin), as lines; `Ok(None)` if no node runs here (`SPIKE_C_NODE`
/// names another binary).
pub fn node_lines(text: &str) -> Result<Option<Vec<String>>, String> {
    let node = std::env::var("SPIKE_C_NODE").unwrap_or_else(|_| "node".to_string());
    let child = Command::new(&node)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{node}: {e}")),
    };
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(text.as_bytes())
        .map_err(|e| format!("{node} stdin: {e}"))?;
    let o = child
        .wait_with_output()
        .map_err(|e| format!("{node}: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "{node} exit {:?}: {}",
            o.status.code(),
            String::from_utf8_lossy(&o.stderr)
        ));
    }
    let s = String::from_utf8(o.stdout).map_err(|e| format!("{node} output: {e}"))?;
    Ok(Some(s.lines().map(str::to_string).collect()))
}

/// The result of one gate kernel at n = 1.
#[derive(Clone, Debug)]
pub struct KernelRun {
    pub kernel: &'static str,
    pub function: &'static str,
    /// T0 vs node: Some(equal), or None if no node runs here.
    pub node: Option<bool>,
    pub t0_lines: usize,
    /// The unforced gate verdict (tier::unforced_ok): equal to T0, 0 exits.
    pub unforced: Result<(), String>,
    pub forcing: Forcing,
    pub ops: BTreeMap<u32, String>,
}

impl KernelRun {
    pub fn ok(&self) -> bool {
        self.node != Some(false)
            && self.unforced.is_ok()
            && self.forcing.bad.is_empty()
            && self.forcing.never().is_empty()
            && !self.forcing.guards.is_empty()
    }

    pub fn lines(&self) -> Vec<String> {
        let never: Vec<String> = self
            .forcing
            .never()
            .iter()
            .map(|g| format!("g{g}"))
            .collect();
        vec![
            format!(
                "{} {} n = 1: T0 vs node {}; config C unforced {}",
                self.kernel,
                self.function,
                match self.node {
                    Some(true) => format!("equal ({} lines)", self.t0_lines),
                    Some(false) => "DIFFERS".to_string(),
                    None => "SKIP (no node on PATH)".to_string(),
                },
                match &self.unforced {
                    Ok(()) => "equal to T0, 0 exits".to_string(),
                    Err(e) => format!("FAIL: {e}"),
                }
            ),
            format!(
                "{} {}: forced {} of {} guards in {} runs, each output equal to T0{}{}",
                self.kernel,
                self.function,
                self.forcing.forced.len(),
                self.forcing.guards.len(),
                self.forcing.runs,
                if never.is_empty() {
                    String::new()
                } else {
                    format!("; NEVER FORCED {}", never.join(" "))
                },
                if self.forcing.bad.is_empty() {
                    String::new()
                } else {
                    format!("; {}", self.forcing.bad.join("; "))
                },
            ),
        ]
    }
}

/// The differential of the gate kernel `k` at n = 1 (module doc).
pub fn run_kernel(k: &'static Kernel, seed: u64) -> Result<KernelRun, String> {
    let config = Config::default();
    let d = kernels::driver(k, 1)?;
    lint(&d.text).map_err(|e| format!("{}: {e}", k.name))?;
    let plain = tier::run_full(k, 1, &config, IntFacts::Live, &EmitOptions::default())?;
    let node = node_lines(&d.text)?.map(|lines| lines == plain.t0.output);
    let unforced = tier::unforced_ok(&plain, false);
    let guards: BTreeSet<u32> = plain
        .report
        .emit
        .meta
        .exits
        .iter()
        .map(|e| e.guard)
        .collect();
    let forcing = force_all(seed, guards, |t| {
        let opts = EmitOptions {
            force: Some(t),
            ..Default::default()
        };
        let r = tier::run_full(k, 1, &config, IntFacts::Live, &opts)?;
        Ok((r.equal, r.report, r.exits))
    })?;
    Ok(KernelRun {
        kernel: k.name,
        function: plain.function,
        node,
        t0_lines: plain.t0.output.len(),
        unforced,
        forcing,
        ops: guard_ops(&plain.report),
    })
}

/// Per guard op: (guards, forced, natural exits) over `runs` (the summary table).
pub fn by_op(
    rows: impl Iterator<Item = (BTreeMap<u32, String>, BTreeMap<u32, u64>, Forcing)>,
) -> BTreeMap<String, (usize, usize, u64)> {
    let mut m: BTreeMap<String, (usize, usize, u64)> = BTreeMap::new();
    for (ops, natural, f) in rows {
        for g in &f.guards {
            let op = ops.get(g).cloned().unwrap_or_else(|| "?".to_string());
            let e = m.entry(op).or_default();
            e.0 += 1;
            e.1 += usize::from(f.forced.contains(g));
            e.2 += natural.get(g).copied().unwrap_or(0);
        }
    }
    m
}

/// `spike-c diff` (module doc): the report on stdout, Ok(true) if every part passed.
pub fn main(args: &[String]) -> Result<bool, String> {
    let mut seeds = SEEDS;
    let mut kernels_on = true;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seeds" => {
                let v = it.next().ok_or("--seeds needs A..B")?;
                let (a, b) = v.split_once("..").ok_or("--seeds A..B")?;
                let a: u64 = a.parse().map_err(|_| format!("bad seed {a}"))?;
                let b: u64 = b.parse().map_err(|_| format!("bad seed {b}"))?;
                if a == 0 || b < a {
                    return Err(format!("bad seed range {v}"));
                }
                seeds = a..=b;
            }
            "--no-kernels" => kernels_on = false,
            "--show" => {
                let v = it.next().ok_or("--show needs a seed")?;
                let s: u64 = v.parse().map_err(|_| format!("bad seed {v}"))?;
                let p = program(s);
                print!("{}", p.text());
                // The graph too, where the emitter runs.
                if spike_b::emit::Arch::HOST == Some(spike_b::emit::Arch::Aarch64) {
                    let c = tier::compiled_script(
                        &Config::default(),
                        &p.defs,
                        &p.warm,
                        &p.main,
                        p.name,
                        IntFacts::Live,
                        &EmitOptions::default(),
                    )?;
                    print!("{}", c.report.graph);
                    for l in c.output.output {
                        println!("| {l}");
                    }
                }
                return Ok(true);
            }
            _ => return Err(format!("unknown option {a}")),
        }
    }
    if spike_b::emit::Arch::HOST != Some(spike_b::emit::Arch::Aarch64) {
        println!("diff: SKIP (the emitter is aarch64 only, protocol §1)");
        return Ok(true);
    }
    let mut ok = true;
    let mut rows = Vec::new();
    if kernels_on {
        if !kernels::bench_present() {
            println!(
                "diff: the gate kernels need the v8-v7 checkout (bash scripts/sync-bench.sh bench-v8v7)"
            );
            ok = false;
        } else {
            for name in KERNELS {
                let k = kernels::kernel(name).ok_or("no kernel")?;
                let r = run_kernel(k, 1)?;
                for l in r.lines() {
                    println!("{l}");
                }
                ok &= r.ok();
                rows.push((r.ops.clone(), BTreeMap::new(), r.forcing.clone()));
            }
        }
    }
    let config = Config::default();
    let (mut pass, mut total, mut guards, mut forced) = (0, 0, 0, 0);
    let mut by_family: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for seed in seeds.clone() {
        let r = run_program(seed, &config);
        println!("{}", r.line());
        total += 1;
        let e = by_family.entry(r.program.family).or_default();
        e.0 += 1;
        if r.ok() {
            pass += 1;
            e.1 += 1;
        } else {
            ok = false;
        }
        guards += r.forcing.guards.len();
        forced += r.forcing.forced.len();
        if let Some(rep) = &r.report {
            rows.push((guard_ops(rep), r.natural.clone(), r.forcing.clone()));
        }
    }
    println!(
        "corpus seeds {}..{}: {pass} of {total} programs right (output equal to T0 unforced and in every forced run, every guard forced); {forced} of {guards} guards forced",
        seeds.start(),
        seeds.end()
    );
    let fams: Vec<String> = by_family
        .iter()
        .map(|(f, (n, p))| format!("{f} {p}/{n}"))
        .collect();
    println!("by family: {}", fams.join(", "));
    println!("exits per guard op (kernels and corpus): guards, forced, natural exits");
    for (op, (g, f, n)) in by_op(rows.into_iter()) {
        println!("  {op:<16} {g:>6} {f:>6} {n:>6}");
    }
    println!("tier diff: {}", if ok { "PASS" } else { "FAIL" });
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Codex M5-P3 r1 spikec7 2: `S` (PRELUDE) shows what print's ToString merges: -0 and
    /// 0, a hole, null and undefined, two objects, the array itself; and the copy family
    /// stores -0 into a DOUBLE array and named objects, so a store that loses one of them
    /// fails the compare (negative control: a store that writes +0 for -0 into raw f64 fails
    /// 15 of the copy seeds in 1..200; it passed all 200 when the arrays were printed).
    #[test]
    fn the_array_form_shows_what_tostring_merges() {
        let main = "var a = [0, -0, , null, undefined, O1, O2, {}, '1', 1.5]; a.push(a); print(S(a)); print(String([0, -0, , null, undefined]));";
        let out = tier::t0_script(&Config::default(), PRELUDE, "", main).expect("t0");
        assert!(out.result.is_ok(), "{out:?}");
        assert_eq!(
            out.output,
            [
                "len 11 number:0 -0 hole object:null undefined:undefined r0 r1 n0 string:1 number:1.5 self",
                "0,0,,,"
            ]
        );
        let copies: Vec<Program> = (1..=200)
            .map(program)
            .filter(|p| p.family == "copy")
            .collect();
        assert!(copies.iter().any(|p| {
            p.main
                .contains("S(f(K([0.5, 0.5, 0.5, 0.5, 0.5]), K([1, -0, 3]), 3))")
        }));
        assert!(copies.iter().all(|p| !p.main.contains("print(f(")));
    }

    /// Codex M5-P3 r2 spk 2, negative controls of the array form as the corpus prints it
    /// (`shown`): the source array in place of an input object, a new object with the
    /// content of `O1` in place of `O1`, and a change past index 255 of a DICTIONARY array
    /// (at 100000, and a new index 300) each print another line.
    #[test]
    fn the_array_form_sees_identity_and_far_indices() {
        let config = Config::default();
        let run = |f: &str, call: &str| {
            let defs = format!("{PRELUDE}{f}\n");
            let out = tier::t0_script(&config, &defs, "", &line(&shown(call))).expect("t0");
            assert!(out.result.is_ok(), "{out:?}");
            out.output
        };
        let copy = "function f(a, b, n) { for (var i = 0; i < n; i++) { a[i] = b[i]; } return a; }";
        let swap = "function f(a, b, n) { for (var i = 0; i < n; i++) { a[i] = i === 1 ? b : b[i]; } return a; }";
        let call = "f([0, 0], [O2, {}, 3], 3)";
        assert_ne!(
            run(copy, call),
            run(swap, call),
            "the source array for {{}}"
        );
        let same = "function f(a) { return [a[0]]; }";
        let twin = "function f(a) { return [{ id: a[0].id }]; }";
        assert_ne!(run(same, "f([O1])"), run(twin, "f([O1])"), "a twin of O1");
        let keep = "function f(a) { return a; }";
        for far in ["a[100000] = 7;", "a[300] = 7;"] {
            let mutant = format!("function f(a) {{ {far} return a; }}");
            let d = "f(D([1, 2, 3]))";
            assert_ne!(run(keep, d), run(&mutant, d), "{far}");
        }
    }

    /// MC-w17-10: the lint refuses each GC-timing name, in code, in a string and as a
    /// property, and passes names that only contain one.
    #[test]
    fn the_lint_refuses_gc_timing_programs() {
        for bad in [
            "var w = new WeakRef({}); print(w.deref() === undefined);",
            "var r = new FinalizationRegistry(function () { print('cb'); });",
            "gc(); print(1);",
            "var g = globalThis['gc']; g();",
            "print(typeof WeakRef);",
            "// a comment that names gc\nprint(1);",
        ] {
            let e = lint(bad).expect_err(bad);
            assert!(e.contains("MC-w17-10"), "{e}");
        }
        for good in [
            "var gcount = 1; function agc() {} var WeakRefs = 2; print(gcount + WeakRefs);",
            "var m = new WeakMap(); var s = new WeakSet(); print(1);",
        ] {
            assert_eq!(lint(good), Ok(()), "{good}");
        }
    }

    /// The generator is deterministic, every program of 1..=200 passes the lint, and the
    /// seeds cover every family.
    #[test]
    fn the_corpus_is_deterministic_and_lint_clean() {
        let mut fams = BTreeSet::new();
        for s in SEEDS {
            let p = program(s);
            assert_eq!(p.text(), program(s).text(), "seed {s}");
            assert_eq!(lint(&p.text()), Ok(()), "seed {s}:\n{}", p.text());
            fams.insert(p.family);
        }
        assert_eq!(fams.len(), FAMILIES.len());
        assert_ne!(program(1).text(), program(1 + FAMILIES.len() as u64).text());
    }

    /// The kernel drivers pass the lint (the kernels name no GC-timing API).
    #[test]
    fn the_kernel_drivers_pass_the_lint() {
        if !kernels::bench_present() {
            eprintln!("skip: no v8-v7 checkout");
            return;
        }
        for name in KERNELS {
            let k = kernels::kernel(name).expect("kernel");
            let d = kernels::driver(k, 1).expect("driver");
            assert_eq!(lint(&d.text), Ok(()), "{name}");
        }
    }

    /// The guard op table reads the graph's guard lines.
    #[test]
    fn guard_ops_reads_the_graph() {
        if spike_b::emit::Arch::HOST != Some(spike_b::emit::Arch::Aarch64) {
            return;
        }
        let r = run_program(1, &Config::default());
        let ops = guard_ops(r.report.as_ref().expect("a compile"));
        assert!(!ops.is_empty());
        for g in &r.forcing.guards {
            assert!(ops.contains_key(g), "g{g}: {ops:?}");
        }
        assert!(ops.values().any(|o| o == "CheckArray"), "{ops:?}");
    }

    /// A handful of seeds (one per family) pass the whole method: equal unforced, every guard
    /// forced, every forced run equal (the gate runs 1..=200).
    #[test]
    fn one_program_per_family_passes() {
        if spike_b::emit::Arch::HOST != Some(spike_b::emit::Arch::Aarch64) {
            return;
        }
        for s in 1..=FAMILIES.len() as u64 {
            let r = run_program(s, &Config::default());
            assert!(r.ok(), "{}\n{}", r.line(), r.program.text());
        }
    }

    /// The method fails when a guard is never forced: a guard on a path `main` never takes.
    #[test]
    fn a_guard_never_reached_fails_the_method() {
        if spike_b::emit::Arch::HOST != Some(spike_b::emit::Arch::Aarch64) {
            return;
        }
        let defs = "function f(a, n, t) { var s = 0; if (t === 1) { for (var i = 0; i < n; i++) { s = (s + a[i]) | 0; } } return s; }";
        let warm = "for (var q = 0; q < 20; q++) { f([1, 2], 2, 1); f([1, 2], 2, 0); }";
        // `main` never takes the loop: its guards stay unforced.
        let main = "print(f([1, 2], 2, 0));";
        let config = Config::default();
        let t0 = tier::t0_script(&config, defs, warm, main).expect("t0");
        let plain = tier::compiled_script(
            &config,
            defs,
            warm,
            main,
            "f",
            IntFacts::Live,
            &EmitOptions::default(),
        )
        .expect("compile");
        let guards: BTreeSet<u32> = plain
            .report
            .emit
            .meta
            .exits
            .iter()
            .map(|e| e.guard)
            .collect();
        let f = force_all(3, guards, |t| {
            let opts = EmitOptions {
                force: Some(t),
                ..Default::default()
            };
            let c = tier::compiled_script(&config, defs, warm, main, "f", IntFacts::Live, &opts)?;
            Ok((c.output == t0, c.report, c.exits))
        })
        .expect("forcing");
        assert!(f.bad.is_empty(), "{:?}", f.bad);
        assert!(!f.never().is_empty(), "{f:?}");
    }
}
