//! Tests of literal sites (`literal_site.rs`) (owner M4-P1 OBJ-ALLOC). Programs run under
//! every dispatch expansion, with and without GC stress, with eager feedback vectors (threshold
//! 0), so the second run of a site can hit. A Literal slot's count counts the cold results
//! only, so a count of 1 after many runs means that the others hit.

use ep9_bytecode::{FeedbackSlot, FeedbackSlotKind, Oddball, VerifiedUnit};

use super::{Asm, config, isolate, on_big_stack, take_output, unit};
use crate::feedback;
use crate::frame::{HOLE, UNDEFINED, int32_bits};
use crate::ic::{self, hk};
use crate::kinds;
use crate::literal_site::{ch, handler_i32};
use crate::object;
use crate::shape;
use crate::{Dispatch, Isolate, IsolateConfig};

/// Bit 0 of the literal flags: shallow (no nested boilerplate).
const SHALLOW: u8 = 1;

/// Every dispatch expansion, with and without GC stress, eager vectors.
fn configs() -> Vec<(String, IsolateConfig)> {
    let mut v = Vec::new();
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let c = IsolateConfig {
                feedback_threshold: 0,
                ..config(d, stress)
            };
            v.push((format!("{} stress={stress}", d.name()), c));
        }
    }
    v
}

/// The (key, value) of slot `slot` of the script's vector (function 0 of the last unit).
fn slot_of(iso: &Isolate, slot: FeedbackSlot) -> (u64, u64) {
    let units = iso.rt.units.borrow();
    let fi = &units.last().unwrap().infos[0];
    let v = feedback::vector(fi).expect("a vector (threshold 0)");
    (feedback::key(v, slot.0), feedback::value(v, slot.0))
}

type Make = fn() -> (VerifiedUnit, Vec<FeedbackSlot>);

/// Runs `make()` in every config, checks the output against `want`, and calls `check` with
/// the isolate and the slots after each run.
fn run_checked(make: Make, want: &'static [&'static str], check: fn(&Isolate, &[FeedbackSlot])) {
    for (label, c) in configs() {
        on_big_stack(move || {
            let mut iso = isolate(c);
            take_output();
            let (u, slots) = make();
            let r = iso.with_cx_checkpoint(|cx| cx.run_script(u, "test.js").map(|_| ()));
            let out = take_output();
            if let Err(e) = r {
                panic!("{label}: uncaught {e}");
            }
            assert_eq!(out, want, "{label}");
            check(&iso, &slots);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// `for (i = 0; i < n; i++) { body }` in `s`; the body gets the counter register.
fn repeat(s: &mut Asm, n: i32, body: impl FnOnce(&mut Asm, ep9_bytecode::Register)) {
    let i = s.b.new_register();
    s.b.lda_zero().star(i);
    let head = s.b.bound_label();
    let exit = s.b.new_label();
    s.b.lda_smi(n).test_less_than(i, s.cmp).jump_if_false(exit);
    body(s, i);
    s.b.ldar(i).inc(s.bin).star(i).jump_loop(head, 0, s.loop_);
    s.b.bind(exit);
}

/// 20 times: `o = {a: 1, b: "s", c: i}` (shallow, `c` computed); then print o.a + o.c, o.b,
/// and the property count of the shape through a `for-in`-free check: `o.d` is undefined.
fn object_literal() -> (VerifiedUnit, Vec<FeedbackSlot>) {
    let mut s = Asm::script(0);
    let lit = s.b.feedback_slot(FeedbackSlotKind::Literal);
    let def = s.b.feedback_slot(FeedbackSlotKind::DefineNamedOwn);
    let (ka, kb, kc) = (
        s.b.string_constant("a"),
        s.b.string_constant("b"),
        s.b.string_constant("c"),
    );
    let (k1, ks) = (s.b.number_constant(1.0), s.b.string_constant("s"));
    let bp =
        s.b.object_boilerplate(vec![(ka, Some(k1)), (kb, Some(ks)), (kc, None)]);
    let o = s.b.new_register();
    repeat(&mut s, 20, |s, i| {
        s.b.create_object_literal(bp, lit, SHALLOW).star(o);
        s.b.ldar(i).define_named_own_property(o, kc, def);
    });
    let t = s.b.new_register();
    s.get_named(o, "a");
    s.b.star(t);
    s.get_named(o, "c");
    s.b.add(t, s.bin);
    s.print_acc();
    s.get_named(o, "b");
    s.print_acc();
    s.get_named(o, "d");
    s.print_acc();
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), vec![lit])
}

#[test]
fn a_shallow_object_literal_hits_after_its_first_run() {
    run_checked(object_literal, &["20", "s", "undefined"], |iso, slots| {
        let (key, value) = slot_of(iso, slots[0]);
        assert!(shape::is_shape(key), "the key is the result shape");
        assert_eq!(shape::count(key), 3);
        // One cold run (the first); the other 19 hit.
        assert_eq!(value, int32_bits(1));
    });
}

/// 20 times: `a = [1, , "s", i]` (shallow: an elision and a computed element) and `e = []`;
/// then print a.length, a[0] + a[3], `1 in a`, a[2], e.length.
fn array_literal() -> (VerifiedUnit, Vec<FeedbackSlot>) {
    let mut s = Asm::script(0);
    let (lit, empty) = (
        s.b.feedback_slot(FeedbackSlotKind::Literal),
        s.b.feedback_slot(FeedbackSlotKind::Literal),
    );
    let sta = s.b.feedback_slot(FeedbackSlotKind::StoreInArrayLiteral);
    let has = s.b.feedback_slot(FeedbackSlotKind::HasKeyed);
    let (k1, ks) = (s.b.number_constant(1.0), s.b.string_constant("s"));
    let hole = s.b.oddball_constant(Oddball::Hole);
    let bp =
        s.b.array_boilerplate(vec![Some(k1), Some(hole), Some(ks), None]);
    let (a, e, idx, t) = (
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
        s.b.new_register(),
    );
    repeat(&mut s, 20, |s, i| {
        s.b.create_array_literal(bp, lit, SHALLOW).star(a);
        s.b.lda_smi(3).star(idx);
        s.b.ldar(i).sta_in_array_literal(a, idx, sta);
        s.b.create_empty_array_literal(empty).star(e);
    });
    s.get_named(a, "length");
    s.print_acc();
    s.b.lda_zero().get_keyed_property(a, s.keyed).star(t);
    s.b.lda_smi(3).get_keyed_property(a, s.keyed).add(t, s.bin);
    s.print_acc();
    s.b.lda_smi(1).star(idx).ldar(a).test_in(idx, has);
    s.print_acc();
    s.b.lda_smi(2).get_keyed_property(a, s.keyed);
    s.print_acc();
    s.get_named(e, "length");
    s.print_acc();
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), vec![lit, empty])
}

#[test]
fn shallow_and_empty_array_literals_hit_after_their_first_run() {
    run_checked(
        array_literal,
        &["4", "20", "false", "s", "0"],
        |iso, slots| {
            for &slot in slots {
                let (key, value) = slot_of(iso, slot);
                assert!(shape::is_shape(key));
                assert_eq!(shape::object_kind(key), kinds::ARRAY.get());
                assert_eq!(value, int32_bits(1));
            }
        },
    );
}

/// 5 times: `o = {x: [1, 2], y: 3}` (not shallow: a nested boilerplate); print o.x[1] + o.y.
fn nested_literal() -> (VerifiedUnit, Vec<FeedbackSlot>) {
    nested_literal_flags(0)
}

/// [`nested_literal`] with the literal flags `flags`.
fn nested_literal_flags(flags: u8) -> (VerifiedUnit, Vec<FeedbackSlot>) {
    let mut s = Asm::script(0);
    let lit = s.b.feedback_slot(FeedbackSlotKind::Literal);
    let (kx, ky) = (s.b.string_constant("x"), s.b.string_constant("y"));
    let (k1, k2, k3) = (
        s.b.number_constant(1.0),
        s.b.number_constant(2.0),
        s.b.number_constant(3.0),
    );
    let inner = s.b.array_boilerplate(vec![Some(k1), Some(k2)]);
    let bp =
        s.b.object_boilerplate(vec![(kx, Some(inner)), (ky, Some(k3))]);
    let (o, x, t) = (s.b.new_register(), s.b.new_register(), s.b.new_register());
    repeat(&mut s, 5, |s, _| {
        s.b.create_object_literal(bp, lit, flags).star(o);
    });
    s.get_named(o, "x");
    s.b.star(x)
        .lda_smi(1)
        .get_keyed_property(x, s.keyed)
        .star(t);
    s.get_named(o, "y");
    s.b.add(t, s.bin);
    s.print_acc();
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), vec![lit])
}

#[test]
fn a_nested_literal_stays_on_the_cold_path_and_is_built_from_its_shape() {
    run_checked(nested_literal, &["5"], |iso, slots| {
        let (key, value) = slot_of(iso, slots[0]);
        assert!(shape::is_shape(key), "the facts get the shape");
        assert_eq!(value, int32_bits(5), "every run was cold");
        // The first run was generic; the other 4 built from the recorded shape.
        assert_eq!(crate::literal_site::COLD_BUILDS.with(|c| c.get()), 4);
    });
}

#[test]
fn a_wrong_shallow_flag_gets_no_key() {
    // The verifier does not check the flag; the miss does, so no hit trusts it.
    run_checked(
        || nested_literal_flags(SHALLOW),
        &["5"],
        |iso, slots| {
            assert_eq!(slot_of(iso, slots[0]), (UNDEFINED, int32_bits(5)));
        },
    );
}

/// Two literal sites share one Literal slot (the verifier allows it; BytecodeGen never does
/// it): `p = {a: 1}` and `q = {b: 2}`, 6 times each; print p.a, q.b, p.b, q.a.
fn shared_slot() -> (VerifiedUnit, Vec<FeedbackSlot>) {
    let mut s = Asm::script(0);
    let lit = s.b.feedback_slot(FeedbackSlotKind::Literal);
    let (ka, kb) = (s.b.string_constant("a"), s.b.string_constant("b"));
    let (k1, k2) = (s.b.number_constant(1.0), s.b.number_constant(2.0));
    let bp = s.b.object_boilerplate(vec![(ka, Some(k1))]);
    let bq = s.b.object_boilerplate(vec![(kb, Some(k2))]);
    let (p, q) = (s.b.new_register(), s.b.new_register());
    repeat(&mut s, 6, |s, _| {
        s.b.create_object_literal(bp, lit, SHALLOW).star(p);
        s.b.create_object_literal(bq, lit, SHALLOW).star(q);
    });
    for (r, name) in [(p, "a"), (q, "b"), (p, "b"), (q, "a")] {
        s.get_named(r, name);
        s.print_acc();
    }
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), vec![lit])
}

#[test]
fn a_shared_literal_slot_gets_no_key() {
    run_checked(
        shared_slot,
        &["1", "2", "undefined", "undefined"],
        |iso, slots| {
            let (key, value) = slot_of(iso, slots[0]);
            assert_eq!(key, UNDEFINED);
            assert_eq!(value, int32_bits(12));
        },
    );
}

/// Array sites and object sites of [`many_sites`].
const MANY_ARRAYS: usize = 1_000;
const MANY_OBJECTS: usize = 250;

/// Straight-line top-level code (no loop): `a = [1, 2]` at [`MANY_ARRAYS`] sites, then
/// `o = {p: 1}` at [`MANY_OBJECTS`] sites, each with its own Literal slot, each run once (the
/// shape of a deep or long script); then print a.length and o.p.
fn many_sites() -> (VerifiedUnit, Vec<FeedbackSlot>) {
    let mut s = Asm::script(0);
    let (k1, k2, kp) = (
        s.b.number_constant(1.0),
        s.b.number_constant(2.0),
        s.b.string_constant("p"),
    );
    let abp = s.b.array_boilerplate(vec![Some(k1), Some(k2)]);
    let obp = s.b.object_boilerplate(vec![(kp, Some(k1))]);
    let (a, o) = (s.b.new_register(), s.b.new_register());
    let mut slots = Vec::new();
    for _ in 0..MANY_ARRAYS {
        let lit = s.b.feedback_slot(FeedbackSlotKind::Literal);
        s.b.create_array_literal(abp, lit, SHALLOW).star(a);
        slots.push(lit);
    }
    for _ in 0..MANY_OBJECTS {
        let lit = s.b.feedback_slot(FeedbackSlotKind::Literal);
        s.b.create_object_literal(obp, lit, SHALLOW).star(o);
        slots.push(lit);
    }
    s.get_named(a, "length");
    s.print_acc();
    s.get_named(o, "p");
    s.print_acc();
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), slots)
}

/// The miss path asks once per site whether its slot is shared (`literal_site::shared_slot`).
/// The answer costs one walk of the function's code in all, not one per miss: the old walk
/// per miss decoded about sites x instructions (1,250 x ~2,500 here), which made deep or long
/// top-level scripts quadratic once a script got its vector at entry (M4-P2 nesting_caps).
/// Every site still records its key. No GC stress (a GC per allocation makes it slow; the
/// walk allocates nothing); every dispatch expansion with eager vectors, and the default
/// threshold (a script gets its vector at entry).
#[test]
fn literal_misses_walk_the_code_once_per_function() {
    let mut cs: Vec<_> = configs()
        .into_iter()
        .filter(|(_, c)| c.heap.gc_stress == 0)
        .collect();
    cs.push(("default threshold".into(), config(Dispatch::ALL[0], false)));
    for (label, c) in cs {
        on_big_stack(move || {
            let (u, slots) = many_sites();
            let instructions =
                ep9_bytecode::Instructions::new(&u.unit().functions[0].bytecode).count() as u64;
            let mut iso = isolate(c);
            take_output();
            crate::literal_site::SCANNED.with(|n| n.set(0));
            let r = iso.with_cx_checkpoint(|cx| cx.run_script(u, "test.js").map(|_| ()));
            let scanned = crate::literal_site::SCANNED.with(|n| n.get());
            let out = take_output();
            if let Err(e) = r {
                panic!("{label}: uncaught {e}");
            }
            assert_eq!(out, ["2", "1"], "{label}");
            assert!(
                scanned <= instructions,
                "{label}: {scanned} instructions decoded for {} misses of a function of \
                 {instructions} instructions (one walk is at most {instructions})",
                slots.len()
            );
            for (n, &slot) in slots.iter().enumerate() {
                let (key, value) = slot_of(&iso, slot);
                assert!(shape::is_shape(key), "{label}: site {n} has a key");
                let want = if n < MANY_ARRAYS {
                    kinds::ARRAY
                } else {
                    kinds::OBJECT
                };
                assert_eq!(shape::object_kind(key), want.get(), "{label}: site {n}");
                assert_eq!(value, int32_bits(1), "{label}: site {n}");
            }
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// For n = 1 ..= `shapes`: `src_n = {}; src_n["x" + n] = n` (one source shape per n; each
/// source stays in its own register, so under GC stress its shape and its case stay alive),
/// then 4 times `c = {...src_n}` at one CloneObject site, and `t += c["x" + n]`; print t.
fn clone_program(shapes: i32) -> (VerifiedUnit, Vec<FeedbackSlot>) {
    let mut s = Asm::script(0);
    let site = s.b.feedback_slot(FeedbackSlotKind::CloneObject);
    let (c, t) = (s.b.new_register(), s.b.new_register());
    s.b.lda_zero().star(t);
    for n in 1..=shapes {
        let name = format!("x{n}");
        let src = s.b.new_register();
        s.b.create_empty_object_literal().star(src);
        s.b.lda_smi(n);
        s.set_named(src, &name);
        for _ in 0..4 {
            s.b.clone_object(src, 0, site).star(c);
        }
        s.get_named(c, &name);
        s.b.add(t, s.bin).star(t);
    }
    s.b.ldar(t);
    s.print_acc();
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), vec![site])
}

#[test]
fn clone_object_goes_monomorphic_polymorphic_and_megamorphic() {
    // One source shape: one cold clone, 3 hits.
    run_checked(
        || clone_program(1),
        &["1"],
        |iso, slots| {
            let (key, value) = slot_of(iso, slots[0]);
            assert!(shape::is_shape(key), "the key is the source shape");
            assert_eq!(handler_i32(value, ch::KIND), hk::CLONE);
            assert_eq!(handler_i32(value, ch::COUNT), 1);
            let result = object::rd(object::cell(value), ch::RESULT);
            assert_eq!(shape::count(result), 1);
        },
    );
    // Three source shapes: polymorphic.
    run_checked(
        || clone_program(3),
        &["6"],
        |iso, slots| {
            let (key, value) = slot_of(iso, slots[0]);
            assert_eq!(key, HOLE);
            assert_eq!(kinds::cell_kind(value), kinds::POLY_IC.get());
        },
    );
    // Five source shapes, one more than a POLY_IC holds (4): megamorphic for good.
    run_checked(
        || clone_program(5),
        &["15"],
        |iso, slots| {
            assert_eq!(slot_of(iso, slots[0]), (HOLE, ic::MEGA));
        },
    );
}

/// `{...src}` with src's properties all in the source's out-of-line storage past its
/// in-object slots: `src = {}` (4 in-object slots) gets 4 + 2 keys; the result (4 in-object
/// slots too) cannot hold 6, so the site records no case.
fn clone_too_big() -> (VerifiedUnit, Vec<FeedbackSlot>) {
    let mut s = Asm::script(0);
    let site = s.b.feedback_slot(FeedbackSlotKind::CloneObject);
    let (src, c) = (s.b.new_register(), s.b.new_register());
    s.b.create_empty_object_literal().star(src);
    for j in 0..6 {
        s.b.lda_smi(j + 1);
        s.set_named(src, &format!("k{j}"));
    }
    for _ in 0..3 {
        s.b.clone_object(src, 0, site).star(c);
    }
    s.get_named(c, "k5");
    s.print_acc();
    s.b.lda_undefined().return_();
    (unit("", vec![s.finish()]), vec![site])
}

#[test]
fn a_clone_past_the_in_object_slots_is_not_cached() {
    run_checked(clone_too_big, &["6"], |iso, slots| {
        assert_eq!(slot_of(iso, slots[0]), (UNDEFINED, int32_bits(0)));
    });
}
