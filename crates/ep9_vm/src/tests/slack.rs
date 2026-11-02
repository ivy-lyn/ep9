//! Tests of slack (`slack.rs`) (owner M5-P2 SLACK; the H17 "Slack tracking" row): the scan
//! of a constructor's `this.<name> =` stores, the per-function memo of its result, and the
//! in-object capacity of the objects `new F` makes. Programs run under every dispatch expansion, with and without
//! GC stress, with eager feedback vectors (threshold 0: the construct site hits from its second
//! construct) and with none (`u32::MAX`: every construct takes the cold path).

use ep9_bytecode::{FeedbackSlotKind, FunctionBuilder, FunctionCode, FunctionId, VerifiedUnit};

use super::{Asm, config, isolate, meta, on_big_stack, take_output, unit};
use crate::kinds;
use crate::shape;
use crate::slack::{self, GENEROUS, MAX_NAMES};
use crate::{Dispatch, Isolate, IsolateConfig};

/// The scan result of `code`.
fn scan(code: &FunctionCode) -> Option<usize> {
    let mut names = [0u32; MAX_NAMES];
    slack::this_store_names(code.bytecode(), &mut names)
}

/// A constructor `name(a, b)` with no feedback slots of its own but the ones `body` asks for.
fn ctor(name: &str, body: impl FnOnce(&mut Asm)) -> FunctionCode {
    let mut a = Asm::new(meta(name, 2, 0, 0, 0));
    body(&mut a);
    a.b.lda_undefined().return_();
    a.finish()
}

/// `function P(x, y) { this.x = x; this.y = y; }`.
fn point(a: &mut Asm) {
    let this = a.b.receiver();
    a.b.ldar(a.b.parameter(0));
    a.set_named(this, "x");
    a.b.ldar(a.b.parameter(1));
    a.set_named(this, "y");
}

#[test]
fn the_scan_counts_distinct_this_stores() {
    assert_eq!(scan(&ctor("P", point)), Some(2));
    // `this.x = a; this.x = b; this.y = 1`: x counts once.
    let twice = ctor("P", |a| {
        let this = a.b.receiver();
        a.b.ldar(a.b.parameter(0));
        a.set_named(this, "x");
        a.b.ldar(a.b.parameter(1));
        a.set_named(this, "x");
        a.b.lda_smi(1);
        a.set_named(this, "y");
    });
    assert_eq!(scan(&twice), Some(2));
    // `var t = this; t.x = a`: followed through a register.
    let through = ctor("P", |a| {
        let (this, t) = (a.b.receiver(), a.b.new_register());
        a.b.mov(this, t);
        a.b.ldar(a.b.parameter(0));
        a.set_named(t, "x");
    });
    assert_eq!(scan(&through), Some(1));
    // `a.x = 1`: a store on another object does not count.
    let other = ctor("P", |a| {
        let p = a.b.parameter(0);
        a.b.lda_smi(1);
        a.set_named(p, "x");
    });
    assert_eq!(scan(&other), Some(0));
    // `this.x` read, then stored: a read keeps `this` in the frame.
    let read = ctor("P", |a| {
        let this = a.b.receiver();
        a.get_named(this, "x");
        a.set_named(this, "y");
    });
    assert_eq!(scan(&read), Some(1));
}

#[test]
fn the_scan_gives_none_when_this_leaves_the_frame() {
    // `this.self = this`: the stored value is `this`.
    let self_store = ctor("E", |a| {
        let this = a.b.receiver();
        a.b.ldar(this);
        a.set_named(this, "self");
    });
    assert_eq!(scan(&self_store), None);
    // `this.a = 1; g(this)`: an argument.
    let call = ctor("E", |a| {
        let this = a.b.receiver();
        a.b.lda_smi(1);
        a.set_named(this, "a");
        let (f, a0) = (a.f, a.a.get(0));
        a.b.mov(this, a0);
        a.lda_global("g");
        a.b.star(f);
        a.b.call_undefined_receiver1(f, a0, a.call);
    });
    assert_eq!(scan(&call), None);
    // `this.m()`: the receiver of a call.
    let method = ctor("E", |a| {
        let this = a.b.receiver();
        let f = a.f;
        a.get_named(this, "m");
        a.b.star(f);
        a.b.call_property0(f, this, a.call);
    });
    assert_eq!(scan(&method), None);
    // `this[k] = 1`: a keyed store.
    let keyed = ctor("E", |a| {
        let this = a.b.receiver();
        let k = a.b.parameter(0);
        a.b.lda_smi(1);
        a.b.set_keyed_property(this, k, a.set_keyed);
    });
    assert_eq!(scan(&keyed), None);
    // A register that held `this` and then got another value no longer counts: `t = this;
    // t = a; g(t)`.
    let overwritten = ctor("P", |a| {
        let (this, t) = (a.b.receiver(), a.b.new_register());
        a.b.mov(this, t);
        a.b.ldar(a.b.parameter(0)).star(t);
        let f = a.f;
        a.lda_global("g");
        a.b.star(f);
        a.b.call_undefined_receiver1(f, t, a.call);
        a.b.lda_smi(1);
        a.set_named(this, "x");
    });
    assert_eq!(scan(&overwritten), Some(1));
}

#[test]
fn more_names_than_the_scan_keeps_give_none() {
    let many = |n: usize| {
        ctor("M", move |a| {
            let this = a.b.receiver();
            for i in 0..n {
                a.b.lda_smi(i as i32);
                a.set_named(this, &format!("f{i}"));
            }
        })
    };
    assert_eq!(scan(&many(MAX_NAMES)), Some(MAX_NAMES));
    assert_eq!(scan(&many(MAX_NAMES + 1)), None);
}

/// What a program left in the globals `o0` (the first instance) and `o` (the last).
struct Seen {
    /// In-object capacity and property count of `o0` and `o`.
    first: (usize, usize),
    last: (usize, usize),
    /// `o0` and `o` have one shape.
    same_shape: bool,
}

/// Runs `make()` in every dispatch expansion, with and without GC stress, with eager
/// vectors and with none; checks the output and returns what each run saw.
fn run_each(make: fn() -> VerifiedUnit, want: &'static [&'static str]) -> Vec<(String, Seen)> {
    let mut seen = Vec::new();
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            for threshold in [0, u32::MAX] {
                let c = IsolateConfig {
                    feedback_threshold: threshold,
                    ..config(d, stress)
                };
                let label = format!("{} stress={stress} threshold={threshold}", d.name());
                let l = label.clone();
                let s = on_big_stack(move || {
                    let mut iso = isolate(c);
                    take_output();
                    let r =
                        iso.with_cx_checkpoint(|cx| cx.run_script(make(), "slack.js").map(|_| ()));
                    let out = take_output();
                    if let Err(e) = r {
                        panic!("{l}: uncaught {e}");
                    }
                    assert_eq!(out, want, "{l}");
                    let s = observe(&iso);
                    iso.collect_garbage();
                    iso.verify_heap().unwrap();
                    s
                });
                seen.push((label, s));
            }
        }
    }
    seen
}

fn observe(iso: &Isolate) -> Seen {
    let read = |name: &str| {
        let o = iso.global_raw(name).unwrap().to_bits();
        assert!(kinds::is_object(o), "{name} is an object");
        let s = shape::shape_of(o);
        (s, (shape::field_layout(s).0, shape::count(s)))
    };
    let (s0, first) = read("o0");
    let (s1, last) = read("o");
    Seen {
        first,
        last,
        same_shape: s0 == s1,
    }
}

/// The script: `P = f1`, then `prelude(s, P)`, then 10 times `o = new P(1, 2)` (`o0` = the
/// first), then `after(s, o)`, and `print(o.x + o.y)`.
fn script(
    p: FunctionCode,
    prelude: impl FnOnce(&mut Asm, ep9_bytecode::Register),
    after: impl FnOnce(&mut Asm, ep9_bytecode::Register),
) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("P");
    let (ctor_reg, o) = (s.b.new_register(), s.b.new_register());
    s.lda_global("P");
    s.b.star(ctor_reg);
    prelude(&mut s, ctor_reg);
    let site = s.b.feedback_slot(FeedbackSlotKind::Call);
    let args = s.b.new_register_list(2);
    let i = s.b.new_register();
    s.b.lda_zero().star(i);
    let head = s.b.bound_label();
    let exit = s.b.new_label();
    s.b.lda_smi(10).test_less_than(i, s.cmp).jump_if_false(exit);
    s.b.lda_smi(1)
        .star(args.get(0))
        .lda_smi(2)
        .star(args.get(1));
    s.lda_global("P");
    s.b.star(ctor_reg);
    s.b.construct(ctor_reg, args, 2, site).star(o);
    s.sta_global("o");
    // `if (i == 0) o0 = o`
    let skip = s.b.new_label();
    s.b.lda_zero()
        .test_equal_strict(i, s.cmp)
        .jump_if_false(skip);
    s.b.ldar(o);
    s.sta_global("o0");
    s.b.bind(skip);
    s.b.ldar(i).inc(s.bin).star(i).jump_loop(head, 0, s.loop_);
    s.b.bind(exit);
    after(&mut s, o);
    let t = s.b.new_register();
    s.get_named(o, "x");
    s.b.star(t);
    s.get_named(o, "y");
    s.b.add(t, s.bin);
    s.print_acc();
    s.b.lda_undefined().return_();
    unit("", vec![s.finish(), p])
}

fn plain_point() -> VerifiedUnit {
    script(ctor("P", point), |_, _| {}, |_, _| {})
}

#[test]
fn new_f_gets_the_store_count_and_one_shape() {
    for (label, s) in run_each(plain_point, &["3"]) {
        // Two stores: two in-object slots (8 before slack), for the first instance too.
        assert_eq!(s.first, (2, 2), "{label}");
        assert_eq!(s.last, (2, 2), "{label}");
        // One shape family: the cold first construct and the hits make the same shape.
        assert!(s.same_shape, "{label}");
    }
}

/// `P.prototype.left = null; P.prototype.x = 0; P.prototype.m = P` before the constructs.
fn point_with_prototype_fields() -> VerifiedUnit {
    script(
        ctor("P", point),
        |s, p| {
            let proto = s.b.new_register();
            s.get_named(p, "prototype");
            s.b.star(proto);
            s.b.lda_null();
            s.set_named(proto, "left");
            s.b.lda_zero();
            s.set_named(proto, "x");
            s.b.ldar(p);
            s.set_named(proto, "m");
        },
        |_, _| {},
    )
}

#[test]
fn prototype_data_fields_add_room_for_their_shadows() {
    for (label, s) in run_each(point_with_prototype_fields, &["3"]) {
        // x, y from the stores; `left` (a data value on the prototype); not `x` again, not
        // `m` and `constructor` (callable).
        assert_eq!(s.first.0, 3, "{label}");
        assert_eq!(s.last, (3, 2), "{label}");
        assert!(s.same_shape, "{label}");
    }
}

/// `function P(x, y) { this.x = x; this.y = y; this.self = this; }`: `this` escapes.
fn escaping_point() -> VerifiedUnit {
    let p = ctor("P", |a| {
        point(a);
        let this = a.b.receiver();
        a.b.ldar(this);
        a.set_named(this, "self");
    });
    script(p, |_, _| {}, |_, _| {})
}

#[test]
fn an_escaping_this_keeps_the_generous_capacity() {
    for (label, s) in run_each(escaping_point, &["3"]) {
        assert_eq!(s.last, (usize::from(GENEROUS), 3), "{label}");
        assert!(s.same_shape, "{label}");
    }
}

/// After the constructs: `o.z = 4; print(o.z)` (a field past the capacity, then a load of
/// it).
fn point_with_a_later_field() -> VerifiedUnit {
    script(
        ctor("P", point),
        |_, _| {},
        |s, o| {
            s.b.lda_smi(4);
            s.set_named(o, "z");
            s.get_named(o, "z");
            s.print_acc();
        },
    )
}

#[test]
fn a_field_added_after_the_constructor_goes_out_of_line() {
    for (label, s) in run_each(point_with_a_later_field, &["4", "3"]) {
        // `o` has 3 properties and keeps its 2 slots (a live object never grows or shrinks
        // in place: H21 P5); the heap check after the run checks every size.
        assert_eq!(s.last, (2, 3), "{label}");
        assert_eq!(s.first, (2, 2), "{label}");
        assert!(!s.same_shape, "{label}");
    }
}

#[test]
fn the_capacity_reads_no_feedback_policy() {
    // Rule 40 / M-R9: with eager vectors (hits) and with none (all cold), every dispatch
    // and GC mode makes the same capacity.
    let seen = run_each(point_with_prototype_fields, &["3"]);
    let (l0, s0) = &seen[0];
    for (label, s) in &seen[1..] {
        assert_eq!((s.first, s.last), (s0.first, s0.last), "{label} vs {l0}");
    }
}

/// A constructor with no store scans to 0 (`instance_capacity` then gives `None`: the
/// generous capacity).
#[test]
fn a_constructor_with_no_store_scans_to_zero() {
    let mut b = FunctionBuilder::new(meta("Z", 0, 0, 0, 0));
    b.lda_undefined().return_();
    assert_eq!(scan(&b.finish().unwrap()), Some(0));
}

/// A constructor of `n` stores with no vector, run 10 times cold in a new thread: the scan
/// runs once (the FunctionInfo keeps the capacity).
#[test]
fn the_capacity_is_chosen_once_per_function() {
    on_big_stack(|| {
        let c = IsolateConfig {
            feedback_threshold: u32::MAX,
            ..config(Dispatch::ALL[0], false)
        };
        let mut iso = isolate(c);
        take_output();
        iso.with_cx_checkpoint(|cx| cx.run_script(plain_point(), "slack.js").map(|_| ()))
            .unwrap();
        assert_eq!(take_output(), ["3"]);
        assert_eq!(slack::SCANS.with(|c| c.get()), 1);
    });
}

/// Like [`script`] with no prelude, with `P.prototype.z = 0` after each construct: the
/// first construct runs before the prototype has `z`, the later ones after.
fn script_proto_grows(p: FunctionCode) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let k = s.b.function_constant(FunctionId(1));
    s.b.create_closure(k, s.closure, 0);
    s.sta_global("P");
    let (ctor_reg, o, proto) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
    let site = s.b.feedback_slot(FeedbackSlotKind::Call);
    let args = s.b.new_register_list(2);
    let i = s.b.new_register();
    s.b.lda_zero().star(i);
    let head = s.b.bound_label();
    let exit = s.b.new_label();
    s.b.lda_smi(10).test_less_than(i, s.cmp).jump_if_false(exit);
    s.b.lda_smi(1)
        .star(args.get(0))
        .lda_smi(2)
        .star(args.get(1));
    s.lda_global("P");
    s.b.star(ctor_reg);
    s.b.construct(ctor_reg, args, 2, site).star(o);
    s.sta_global("o");
    let skip = s.b.new_label();
    s.b.lda_zero()
        .test_equal_strict(i, s.cmp)
        .jump_if_false(skip);
    s.b.ldar(o);
    s.sta_global("o0");
    s.b.bind(skip);
    s.get_named(ctor_reg, "prototype");
    s.b.star(proto);
    s.b.lda_zero();
    s.set_named(proto, "z");
    s.b.ldar(i).inc(s.bin).star(i).jump_loop(head, 0, s.loop_);
    s.b.bind(exit);
    let t = s.b.new_register();
    s.get_named(o, "x");
    s.b.star(t);
    s.get_named(o, "y");
    s.b.add(t, s.bin);
    s.print_acc();
    s.b.lda_undefined().return_();
    unit("", vec![s.finish(), p])
}

#[test]
fn a_prototype_field_added_after_the_first_construct_changes_no_capacity() {
    // Rule 40 / M-R9: the cold path (no vector) and the construct-site hit (eager vector)
    // give the same capacity: the one chosen at the first construct (x, y; no `z` yet).
    let seen = run_each(|| script_proto_grows(ctor("P", point)), &["3"]);
    for (label, s) in &seen {
        assert_eq!(s.first, (2, 2), "{label}");
        assert_eq!(s.last, (2, 2), "{label}");
        assert!(s.same_shape, "{label}");
    }
}

/// A constructor of 0 parameters whose code is `LdaZero; this.<names[k]> = acc;
/// LdaZero x3; LdaSmi i1; LdaZero x2; LdaSmi i2; LdaZero x2; Return`, with the string
/// constants `consts` (slot 0 the store's).
fn colliding_ctor(name: &str, consts: [&str; 2], k: usize, i1: i32, i2: i32) -> FunctionCode {
    let mut b = FunctionBuilder::new(meta(name, 0, 0, 0, 0));
    let slot = b.feedback_slot(FeedbackSlotKind::StoreNamed);
    let ks = consts.map(|c| b.string_constant(c));
    let this = b.receiver();
    b.lda_zero().set_named_property(this, ks[k], slot);
    b.lda_zero().lda_zero().lda_zero().lda_smi(i1);
    b.lda_zero().lda_zero().lda_smi(i2);
    b.lda_zero().lda_zero().return_();
    b.finish().unwrap()
}

/// `A = f1; B = f2; B.prototype.x = null; new A(); o0 = o = new B(); print(1)`.
fn colliding_constructors() -> VerifiedUnit {
    // Two codes of equal length that differ in the stored name (constant 1 in A, 0 in B)
    // and in the LdaSmi immediates, chosen so the two have one 64-bit FNV-1a hash (the
    // M5-P2 SLACK scan cache key, length and hash: Codex round 1 finding, slack 1).
    let a = colliding_ctor("A", ["q", "y"], 1, 0x1234_5678, 0x7654_3210);
    let b = colliding_ctor(
        "B",
        ["x", "y"],
        0,
        0xf873_f47f_u32 as i32,
        0x80d5_dba7_u32 as i32,
    );
    let hex = |c: &FunctionCode| {
        c.bytecode()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    assert_eq!(
        hex(&a),
        "0324ff01000303030204785634120303020410325476030392"
    );
    assert_eq!(
        hex(&b),
        "0324ff000003030302047ff473f803030204a7dbd580030392"
    );
    let mut s = Asm::script(0);
    for (i, g) in [(1, "A"), (2, "B")] {
        let k = s.b.function_constant(FunctionId(i));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(g);
    }
    let (f, proto) = (s.b.new_register(), s.b.new_register());
    s.lda_global("B");
    s.b.star(f);
    s.get_named(f, "prototype");
    s.b.star(proto);
    s.b.lda_null();
    s.set_named(proto, "x");
    let args = s.b.new_register_list(0);
    let site = s.b.feedback_slot(FeedbackSlotKind::Call);
    s.lda_global("A");
    s.b.star(f);
    s.b.construct(f, args, 0, site);
    s.lda_global("B");
    s.b.star(f);
    s.b.construct(f, args, 0, site);
    s.sta_global("o");
    s.sta_global("o0");
    s.b.lda_smi(1);
    s.print_acc();
    s.b.lda_undefined().return_();
    unit("", vec![s.finish(), a, b])
}

#[test]
fn a_constructor_never_reads_another_constructors_scan() {
    // B stores `x`, which B.prototype has as a data field: one slot (x counts once). A
    // capacity from A's names (constant 1 = "y", not on the prototype) would be 2.
    for (label, s) in run_each(colliding_constructors, &["1"]) {
        assert_eq!(s.last, (1, 1), "{label}");
    }
}
