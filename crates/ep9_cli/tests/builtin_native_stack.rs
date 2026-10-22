//! H23 G6 end to end through the `ep9` binary: a builtin that recurses in Rust checks the
//! native stack at each step, so a small isolate stack gives the catchable RangeError
//! "Maximum call stack size exceeded" and the process never aborts.
//!
//! ```text
//! ep9 --stack-size 1|2|4 -e <script>  -> exit 0, stdout "RangeError Maximum call stack ..."
//!                                        then "after" (the script goes on after the catch)
//! ```
//!
//! FlattenIntoArray (`flat`, and `flat` called from a `flatMap` mapper) recurses once per
//! nesting level. A cyclic array nests without end, so only the native stack check (or the
//! 10,000-level cap, far past a 1-4 MiB stack in a debug build) ends it. `flatMap` itself
//! flattens one level (depth 1), so its deep case is a mapper that recurses through
//! `flatMap` (each step a host call, checked by the VM) or calls `flat`. A subprocess, so an
//! abort cannot kill the test binary.

use std::process::Command;

const CASES: &[(&str, &str)] = &[
    (
        "flat on a cyclic array",
        "var a = []; a[0] = a; try { a.flat(Infinity); print('no throw'); } \
         catch (e) { print(e.name, e.message); } print('after');",
    ),
    (
        "flat of a cyclic array in a flatMap mapper",
        "var a = []; a[0] = a; try { [0].flatMap(function () { return a.flat(Infinity); }); \
         print('no throw'); } catch (e) { print(e.name, e.message); } print('after');",
    ),
    (
        "flatMap through a recursive mapper",
        "function f(x) { return [x].flatMap(f); } try { f(0); print('no throw'); } \
         catch (e) { print(e.name, e.message); } print('after');",
    ),
];

#[test]
fn recursive_builtins_throw_a_range_error_on_a_small_stack() {
    let mut failures = Vec::new();
    for mib in [1, 2, 4] {
        for (name, script) in CASES {
            let out = Command::new(env!("CARGO_BIN_EXE_ep9"))
                .args(["--stack-size", &mib.to_string(), "-e", script])
                .output()
                .expect("run ep9");
            let stdout = String::from_utf8_lossy(&out.stdout);
            let expected = "RangeError Maximum call stack size exceeded\nafter\n";
            if out.status.code() != Some(0) || stdout != expected {
                failures.push(format!(
                    "{name}, --stack-size {mib}: status {:?}, stdout {stdout:?}, stderr {:?}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
