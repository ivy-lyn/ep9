//! M0 exit item 7, end to end through the `ep9` binary: for each nesting form, the deepest
//! program the front end accepts (the cap) and one level less run and print `ok`; one level
//! more gives a RangeError report and exit status 1. The process never aborts.
//!
//! ```text
//! form(n) = prefix + n nested levels + "; print('ok')"
//! n_cap   = largest n with ep9_frontend::check_early_errors(form(n)) == Ok   (binary search)
//! ep9 form(n_cap - 1)  -> exit 0, "ok"
//! ep9 form(n_cap)      -> exit 0, "ok"
//! ep9 form(n_cap + 1)  -> exit 1, "RangeError: Maximum call stack size exceeded", no signal
//! ep9 form(REGEX_FAR)  -> the same, for the regex forms
//! ```
//!
//! The forms are those of M0 exit item 7: bracket (array, paren, object, block, function),
//! binary, member, call, comma, template (also tagged) and class (body, heritage); and those of
//! M1 exit item 8 (M4-P2 FE-FUZZ, GFF-DEPTH-E2E): conditional (also nested in the consequent),
//! assignment, unary (`!` and `typeof`), arrow and exponent, the same 7 forms as the `m1_*`
//! tests of ep9_frontend `depth_tests.rs`. The programs also run, so a deep tree that compiles
//! must also execute. The regex forms nest `v`-flag classes, which only the pre-scan limits (a
//! regex has no AST depth).

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use ep9_frontend::{AST_CAP, CompileError, SourceOpts, Strictness, check_early_errors};

struct Form {
    name: &'static str,
    build: fn(usize) -> String,
}

fn wrap(prefix: &str, open: &str, mid: &str, close: &str, n: usize) -> String {
    let mut s = String::with_capacity(prefix.len() + (open.len() + close.len()) * n + 64);
    s.push_str(prefix);
    for _ in 0..n {
        s.push_str(open);
    }
    s.push_str(mid);
    for _ in 0..n {
        s.push_str(close);
    }
    s.push_str(";\nprint('ok');\n");
    s
}

/// A regex literal in a function that never runs (the interpreter has no RegExp yet).
fn regex(open: &str, mid: &str, close: &str, flags: &str, n: usize) -> String {
    let mut s = String::with_capacity((open.len() + close.len()) * n + 64);
    s.push_str("function f() { return /");
    for _ in 0..n {
        s.push_str(open);
    }
    s.push_str(mid);
    for _ in 0..n {
        s.push_str(close);
    }
    s.push('/');
    s.push_str(flags);
    s.push_str("; }\nprint('ok');\n");
    s
}

fn chain(prefix: &str, head: &str, link: &str, n: usize) -> String {
    let mut s = String::with_capacity(prefix.len() + head.len() + link.len() * n + 64);
    s.push_str(prefix);
    s.push_str(head);
    for _ in 0..n {
        s.push_str(link);
    }
    s.push_str(";\nprint('ok');\n");
    s
}

const FORMS: &[Form] = &[
    Form {
        name: "bracket-array",
        build: |n| wrap("var x = ", "[", "0", "]", n),
    },
    Form {
        name: "bracket-paren",
        build: |n| wrap("var x = ", "(", "0", ")", n),
    },
    Form {
        name: "bracket-object",
        build: |n| wrap("var x = ", "{a:", "0", "}", n),
    },
    Form {
        name: "bracket-block",
        build: |n| wrap("", "{", "", "}", n),
    },
    Form {
        name: "bracket-function",
        build: |n| wrap("", "function f(){", "", "}", n),
    },
    Form {
        name: "binary",
        build: |n| chain("var x = ", "1", "+1", n),
    },
    Form {
        name: "member",
        build: |n| chain("var a = {}; a.b = a;\nvar x = ", "a", ".b", n),
    },
    Form {
        name: "call",
        build: |n| chain("function f() { return f; }\nvar x = ", "f", "()", n),
    },
    Form {
        name: "comma-nested",
        build: |n| wrap("var x = ", "(0,", "0", ")", n),
    },
    Form {
        name: "template",
        build: |n| wrap("var x = ", "`${", "0", "}`", n),
    },
    Form {
        name: "tagged-template",
        build: |n| chain("function f() { return f; }\nvar x = ", "f", "`x`", n),
    },
    Form {
        name: "class-body",
        build: |n| wrap("", "class A{m(){", "", "}}", n),
    },
    Form {
        name: "class-heritage",
        build: |n| wrap("var x = ", "class extends ", "Object", "{}", n),
    },
    // M1 exit item 8
    Form {
        name: "conditional",
        build: |n| wrap("var a = 0;\nvar x = ", "a?a:", "a", "", n),
    },
    Form {
        name: "conditional-consequent",
        build: |n| wrap("var a = 0;\nvar x = ", "a?", "a", ":a", n),
    },
    Form {
        name: "assignment",
        build: |n| wrap("var a;\nvar x = ", "a=", "0", "", n),
    },
    Form {
        name: "unary",
        build: |n| wrap("var a = 0;\nvar x = ", "!", "a", "", n),
    },
    Form {
        name: "unary-typeof",
        build: |n| wrap("var a = 0;\nvar x = ", "typeof ", "a", "", n),
    },
    Form {
        name: "arrow",
        build: |n| wrap("var x = ", "x=>", "0", "", n),
    },
    Form {
        name: "exponent",
        build: |n| wrap("var a = 1;\nvar x = ", "a**", "a", "", n),
    },
];

/// The M1 exit item 8 forms (the `m1_*` gate tests of ep9_frontend `depth_tests.rs`).
const M1_FORMS: [&str; 7] = [
    "conditional",
    "conditional-consequent",
    "assignment",
    "unary",
    "unary-typeof",
    "arrow",
    "exponent",
];

/// With the `v` flag a class stays open after a nested class closes: `[[a]&&[[a]&&[a]]]` is 3
/// levels deep although every lexical class `[...]` ends at its first `]`.
const REGEX_FORMS: &[Form] = &[
    Form {
        name: "regex-class-v-intersection",
        build: |n| regex("[[a]&&", "[a]", "]", "v", n),
    },
    Form {
        name: "regex-class-v-subtraction",
        build: |n| regex("[[a]--", "[a]", "]", "v", n),
    },
];

/// Regex levels far above the cap: without the pre-scan they overflow the front-end worker's
/// stack in debug and in release.
const REGEX_FAR: usize = 1_000_000;

fn accepted(src: &str) -> bool {
    match check_early_errors(src, SourceOpts::script(Strictness::Sloppy)) {
        Ok(()) => true,
        Err(CompileError::Range(_)) => false,
        Err(e) => panic!("not a depth error: {e}"),
    }
}

/// The largest `n` whose program the front end accepts.
fn cap_of(form: &Form) -> usize {
    // Every level adds at least one AST level, so AST_CAP + 1 levels are too deep; every form
    // adds at most 4, so AST_CAP / 8 levels are fine. A regex level is 4 pre-scan units, so the
    // cap of a regex form is near PRESCAN_CAP / 4, also in between.
    let (mut lo, mut hi) = (AST_CAP as usize / 8, AST_CAP as usize + 1);
    assert!(
        accepted(&(form.build)(lo)),
        "{}: n={lo} rejected",
        form.name
    );
    assert!(
        !accepted(&(form.build)(hi)),
        "{}: n={hi} accepted",
        form.name
    );
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if accepted(&(form.build)(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

fn run_ep9(form: &Form, n: usize, dir: &std::path::Path) -> std::process::Output {
    let path = dir.join(format!("{}-{n}.js", form.name));
    std::fs::write(&path, (form.build)(n)).expect("write program");
    let out = Command::new(env!("CARGO_BIN_EXE_ep9"))
        .arg(&path)
        .output()
        .expect("run ep9");
    let _ = std::fs::remove_file(&path);
    out
}

fn temp_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("ep9-nesting-caps-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// `f` over `items` on up to `available_parallelism` threads; the results in item order.
/// Every `ep9` run is its own process with its own stacks, so the threads change nothing it
/// checks; each thread here also gets its own front-end worker for `check_early_errors`.
fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(items.len()));
    let threads = std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .min(items.len());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { return };
                    let r = f(item);
                    results.lock().expect("lock").push((i, r));
                }
            });
        }
    });
    let mut results = results.into_inner().expect("lock");
    results.sort_by_key(|&(i, _)| i);
    results.into_iter().map(|(_, r)| r).collect()
}

/// Wall time is a few deep programs that run for minutes in a debug `ep9` (bracket-array,
/// bracket-object, class-heritage), so the runs, not the forms, are the unit of parallel work.
#[test]
fn caps_hold_end_to_end() {
    let dir = temp_dir();
    let regex_forms = REGEX_FORMS.iter().map(|form| (form, Some(REGEX_FAR)));
    let forms: Vec<(&Form, Option<usize>)> = FORMS
        .iter()
        .map(|form| (form, None))
        .chain(regex_forms)
        .collect();
    let caps = par_map(&forms, |&(form, _)| cap_of(form));
    // (form, n, cap, expected to print ok)
    let mut runs = Vec::new();
    for (&(form, far), &cap) in forms.iter().zip(&caps) {
        for n in [cap - 1, cap] {
            runs.push((form, n, cap, true));
        }
        for n in std::iter::once(cap + 1).chain(far) {
            runs.push((form, n, cap, false));
        }
    }
    let outs = par_map(&runs, |&(form, n, _, _)| run_ep9(form, n, &dir));
    let mut failures = Vec::new();
    for (&(form, n, cap, ok), out) in runs.iter().zip(&outs) {
        if ok {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if out.status.code() != Some(0) || stdout != "ok\n" {
                failures.push(format!(
                    "{} n={n} (cap {cap}): status {:?}, stdout {stdout:?}, stderr {}",
                    form.name,
                    out.status,
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if out.status.code() != Some(1)
                || !stderr.contains("RangeError: Maximum call stack size exceeded")
                || !out.stdout.is_empty()
            {
                failures.push(format!(
                    "{} n={n} (cap {cap}): status {:?}, stderr {}",
                    form.name,
                    out.status,
                    stderr.chars().take(400).collect::<String>()
                ));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    let report: Vec<String> = forms
        .iter()
        .zip(&caps)
        .map(|(&(form, _), cap)| format!("{}: cap {cap} levels", form.name))
        .collect();
    println!("{}", report.join("\n"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_m1_form_is_listed() {
    for name in M1_FORMS {
        assert!(
            FORMS.iter().any(|f| f.name == name),
            "{name}: M1 exit item 8 form missing from FORMS"
        );
    }
}
