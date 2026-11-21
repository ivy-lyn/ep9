//! LDAR-ELIDE probe, T4: the stack traces of 15 `this` and parameter cases are the same in
//! arm 0, arm 1 and arm 2 (`EP9_LDAR_ELIDE`; the `ep9` binary reads it once per process). The
//! expected text is the m10-p5 output (jit = jitless, byte-equal), so a dropped dead `Ldar`
//! never moves the line or column of a throw (INV-1).
//!
//! ```text
//! ep9 t3.js   (EP9_LDAR_ELIDE = 0, 1, 2)  ->  stdout = M10P5 (15 lines)
//! ```
//!
//! T7: the slack scan (`ep9_vm` `slack.rs`) reads the bytecode of each constructor. The
//! feedback facts of a constructor workload (result shapes with their slack capacity) are the
//! same in every arm, except the unit content hash.

use std::process::Command;

const T3: &str = r#""use strict";
function show(tag, f) { try { f(); print(tag, "no throw"); } catch (e) { var s = String(e && e.stack).split("\n"); print(tag, "|", s[0], "|", s[1], "|", s[2]); } }
function c1() {
  this.a;
}
function c2() {
  return this.a.b;
}
function c3() {
  this.m(1);
}
function c4() {
  this.a = 1;
}
function c5() {
  var k = 1;
  this
    .a;
}
function c6() {
  try { this.a; } finally { k6 = 1; }
}
var k6;
function c7(p) {
  p.x;
}
function c8(p) {
  p.m();
}
class G { get g() { throw new Error("getter"); } c9() { this.g; } c10() { let v = this.g + 1; return v; } c11(p) { p; this.g; } }
function* c12() { this.a; yield 1; }
function c13(p) { if (p) { this.q.r = 2; } }
function c14(p) { for (var i = 0; i < 1; i++) this.a; }
function c15(p) { p.x = this; this.a; }
show("c1 this.a; (this undefined)", c1);
show("c2 return this.a.b", c2);
show("c3 this.m(1)", c3);
show("c4 this.a = 1", c4);
show("c5 this\\n.a", c5);
show("c6 try this.a", c6);
show("c7 p.x; (param)", c7);
show("c8 p.m(); (param)", c8);
show("c9 this.g getter", () => new G().c9());
show("c10 let v = this.g + 1", () => new G().c10());
show("c11 p; this.g;", () => new G().c11(1));
show("c12 generator this.a", () => c12().next());
show("c13 this.q.r = 2", () => c13.call({}, 1));
show("c14 loop this.a", c14);
show("c15 p.x = this; this.a", () => c15({}));
"#;

/// The m10-p5 output of `ep9 t3.js` (the design's `t3.base.jitless.txt`).
const M10P5: &str = r#"c1 this.a; (this undefined) | TypeError: Cannot read properties of undefined (reading 'a') |     at c1 (t3.js:4:8) |     at show (t3.js:2:31)
c2 return this.a.b | TypeError: Cannot read properties of undefined (reading 'a') |     at c2 (t3.js:7:15) |     at show (t3.js:2:31)
c3 this.m(1) | TypeError: Cannot read properties of undefined (reading 'm') |     at c3 (t3.js:10:8) |     at show (t3.js:2:31)
c4 this.a = 1 | TypeError: Cannot set properties of undefined (setting 'a') |     at c4 (t3.js:13:3) |     at show (t3.js:2:31)
c5 this\n.a | TypeError: Cannot read properties of undefined (reading 'a') |     at c5 (t3.js:18:6) |     at show (t3.js:2:31)
c6 try this.a | TypeError: Cannot read properties of undefined (reading 'a') |     at c6 (t3.js:21:14) |     at show (t3.js:2:31)
c7 p.x; (param) | TypeError: Cannot read properties of undefined (reading 'x') |     at c7 (t3.js:25:3) |     at show (t3.js:2:31)
c8 p.m(); (param) | TypeError: Cannot read properties of undefined (reading 'm') |     at c8 (t3.js:28:5) |     at show (t3.js:2:31)
c9 this.g getter | Error: getter |     at get g (t3.js:30:27) |     at c9 (t3.js:30:62)
c10 let v = this.g + 1 | Error: getter |     at get g (t3.js:30:27) |     at c10 (t3.js:30:88)
c11 p; this.g; | Error: getter |     at get g (t3.js:30:27) |     at c11 (t3.js:30:124)
c12 generator this.a | TypeError: Cannot read properties of undefined (reading 'a') |     at c12 (t3.js:31:24) |     at t3.js:46:42
c13 this.q.r = 2 | TypeError: Cannot set properties of undefined (setting 'r') |     at c13 (t3.js:32:28) |     at t3.js:47:36
c14 loop this.a | TypeError: Cannot read properties of undefined (reading 'a') |     at c14 (t3.js:33:52) |     at show (t3.js:2:31)
c15 p.x = this; this.a | TypeError: Cannot read properties of undefined (reading 'a') |     at c15 (t3.js:34:36) |     at t3.js:49:38
"#;

#[test]
fn stack_traces_are_the_same_in_every_arm() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ldar-elide-t4");
    std::fs::create_dir_all(&dir).expect("make the test dir");
    std::fs::write(dir.join("t3.js"), T3).expect("write t3.js");
    let mut failures = Vec::new();
    for arm in ["0", "1", "2"] {
        let out = Command::new(env!("CARGO_BIN_EXE_ep9"))
            .arg("t3.js")
            .current_dir(&dir)
            .env("EP9_LDAR_ELIDE", arm)
            .output()
            .expect("run ep9");
        let stdout = String::from_utf8_lossy(&out.stdout);
        if out.status.code() != Some(0) || stdout != M10P5 {
            failures.push(format!(
                "arm {arm}: status {:?}\n{stdout}\nstderr {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The committed golden of `gen-basic` (the m10-p5 text; the merge role re-blesses it).
const GEN_BASIC: &str = include_str!("../../ep9_frontend/src/bytecodegen/golden/gen-basic.txt");

/// Arm 0 through the binary is today's output: `ep9 --print-bytecode` prints the committed
/// golden text with the switch not set and with `EP9_LDAR_ELIDE=0`. The arm-1 row shows that
/// the check can tell the arms apart.
#[test]
fn arm0_prints_the_golden_bytecode() {
    let mut parts = GEN_BASIC.splitn(3, '\n');
    let src = parts
        .nth(1)
        .and_then(|l| l.strip_prefix("# | "))
        .expect("the golden source line");
    let want = parts.next().expect("the golden body");
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ldar-elide-arm0");
    std::fs::create_dir_all(&dir).expect("make the test dir");
    std::fs::write(dir.join("gen.js"), src).expect("write gen.js");
    for arm in [None, Some("0"), Some("1")] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_ep9"));
        cmd.args(["--print-bytecode", "gen.js"]).current_dir(&dir);
        match arm {
            None => cmd.env_remove("EP9_LDAR_ELIDE"),
            Some(a) => cmd.env("EP9_LDAR_ELIDE", a),
        };
        let out = cmd.output().expect("run ep9");
        assert_eq!(out.status.code(), Some(0), "arm {arm:?}");
        let got = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            got.strip_suffix('\n') == Some(want),
            arm != Some("1"),
            "arm {arm:?}:\n{got}"
        );
    }
}

/// T7: constructors with `this.<name> =` stores at statement starts (the dropped loads).
const SLACK: &str = r#"function P(x, y) { this.x = x; this.y = y; }
function Q(p) { this.a = 1; this.b = { c: 2 }; p; this.c = p; }
function R(p) { this.a = p; this.m(); }
R.prototype.m = function () { this.z = 3; };
function S(p) { p.x = this; this.k = 1; }
var s = 0;
for (var i = 0; i < 200; i++) {
  var o = new P(i, i + 1), q = new Q(i), r = new R(i), t = new S({});
  s += o.x + q.c + r.z + t.k;
}
print(s);
"#;

/// The facts file with each copy of its unit hash (header bytes 16..48) set to zero.
fn facts_without_unit_hash(bytes: &[u8]) -> Vec<u8> {
    let hash = bytes[16..48].to_vec();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(&hash) {
            out.extend(std::iter::repeat_n(0, hash.len()));
            i += hash.len();
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

#[test]
fn slack_facts_are_the_same_in_every_arm() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ldar-elide-t7");
    std::fs::create_dir_all(&dir).expect("make the test dir");
    std::fs::write(dir.join("slack.js"), SLACK).expect("write slack.js");
    let mut facts = Vec::new();
    for arm in ["0", "1", "2"] {
        let file = format!("facts{arm}.bin");
        let out = Command::new(env!("CARGO_BIN_EXE_ep9"))
            .args(["--facts-out", &file, "slack.js"])
            .current_dir(&dir)
            .env("EP9_LDAR_ELIDE", arm)
            .output()
            .expect("run ep9");
        assert_eq!(out.status.code(), Some(0), "arm {arm}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "40600\n", "arm {arm}");
        let bytes = std::fs::read(dir.join(&file)).expect("read the facts");
        facts.push(facts_without_unit_hash(&bytes));
    }
    assert_eq!(facts[1], facts[0], "arm 1");
    assert_eq!(facts[2], facts[0], "arm 2");
}
