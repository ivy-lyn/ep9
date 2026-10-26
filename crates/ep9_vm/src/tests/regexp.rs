//! Tests of RegExp in the VM (`regexp.rs`, `abi/regexp.rs`) (owner M4-P2 RX0A, then M4-P3
//! REGEXP1). ep9_vm tests cannot compile JS, so they drive `Cx::regexp_create` and
//! `Cx::regexp_exec` from Rust and make literals with the bytecode builder.
//!
//! | Test | Shows |
//! |---|---|
//! | `exec_from_rust` | spans and captures, Latin-1, UTF-16 and rope subjects, the start index, sticky, a start inside a surrogate pair (each also under GC stress) |
//! | `create_errors_and_slots` | the SyntaxErrors of bad flags and patterns, the TypeError of a non-RegExp, the internal slots, `lastIndex` |
//! | `literal_compiles_at_first_exec` | `CreateRegExpLiteral` makes a new object per evaluation; the program index is set by the first exec and kept |
//! | `a_dead_regexp_frees_its_program` | the weak source frees the table entry of a RegExp that died, and the index is reused (V9-A7) |
//! | `exec_runs_a_pending_interrupt` | a requested interrupt runs at the exec's poll, then the match goes on |
//! | `exec_stops_on_termination` | a termination request ends the exec with the uncatchable termination |
//! | `exec_keeps_the_receiver_across_the_flatten` | under GC stress, an exec whose only RegExp root is its argument keeps the program while the rope subject is flattened (fails before the fix in release only) |

use core::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ep9_bytecode::{FeedbackSlotKind, VerifiedUnit};

use super::{Asm, config, isolate, unit};
use crate::abi::{MatchRecord, ObjectClass};
use crate::regexp::{ProgramTable, slot};
use crate::runtime::Runtime;
use crate::{Cx, Dispatch, Isolate, JsObject, Local};

/// Runs `f` with a builtin-ABI context of a fresh isolate, then collects and verifies the
/// heap. `stress`: a collection before every allocation, then the heap verifier.
fn with_cx<R>(stress: bool, f: impl FnOnce(&mut Cx<'_>, &Runtime) -> R) -> R {
    let mut iso = isolate(config(Dispatch::DEFAULT, stress));
    let out = {
        let iso: &mut Isolate = &mut iso;
        let mut cx = Cx::new(&mut iso.cx);
        f(&mut cx, &iso.rt)
    };
    iso.collect_garbage();
    iso.verify_heap().unwrap();
    out
}

/// Takes the pending exception: "Name: message".
fn take_error(cx: &mut Cx<'_>, rt: &Runtime) -> String {
    let e: Local<'_> = Local::from_bits(rt.take_pending());
    let e = e.as_object().expect("an Error object");
    let name = cx.get(e, "name").unwrap();
    let msg = cx.get(e, "message").unwrap();
    format!(
        "{}: {}",
        cx.to_rust_string(name).unwrap(),
        cx.to_rust_string(msg).unwrap()
    )
}

/// A RegExp with pattern `p` and flags `f` (prototype: a new ordinary object).
fn re<'s>(cx: &mut Cx<'s>, p: &str, f: &str) -> Local<'s, JsObject> {
    let proto = cx.new_object();
    let (p, f) = (cx.new_string(p).unwrap(), cx.new_string(f).unwrap());
    cx.regexp_create(proto, p, f).unwrap()
}

/// The spans of a match as text: "s-e" per capture, "-" for one that did not take part.
fn spans(m: &MatchRecord) -> String {
    (0..=m.group_count())
        .map(|i| m.get(i).map_or("-".into(), |(s, e)| format!("{s}-{e}")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One exec of `r` on the UTF-16 text `s` from `i`: its spans, or "null".
fn exec16<'s>(cx: &mut Cx<'s>, r: Local<'s, JsObject>, s: &[u16], i: usize) -> String {
    let s = cx.new_string_utf16(s).unwrap();
    match cx.regexp_exec(r, s, i).unwrap() {
        Some(m) => spans(&m),
        None => "null".into(),
    }
}

fn exec<'s>(cx: &mut Cx<'s>, r: Local<'s, JsObject>, s: &str, i: usize) -> String {
    exec16(cx, r, &s.encode_utf16().collect::<Vec<_>>(), i)
}

#[test]
fn exec_from_rust() {
    for stress in [false, true] {
        with_cx(stress, |cx, rt| {
            let r = re(cx, "(a)(x)?(b+)", "");
            assert_eq!(exec(cx, r, "zzabbz", 0), "2-5 2-3 - 3-5");
            assert_eq!(exec(cx, r, "zzabbz", 3), "null", "the start index");
            assert_eq!(exec(cx, r, "zzabbz", 99), "null", "start > length");
            let m = {
                let s = cx.new_string("..abb").unwrap();
                cx.regexp_exec(r, s, 0).unwrap().unwrap()
            };
            assert_eq!((m.start(), m.end(), m.group_count()), (2, 5, 3));
            assert_eq!((m.get(2), m.get(4)), (None, None));
            // Latin-1 above 0x7f, and UTF-16.
            let r = re(cx, "\u{e9}+", "i");
            assert_eq!(exec(cx, r, "caf\u{c9}\u{e9}", 0), "3-5");
            assert_eq!(exec(cx, r, "\u{4e2d}\u{e9}", 0), "1-2");
            // A rope subject (CONS) is flattened first.
            let a = object_string(rt, "the quick brown fox jumps over ");
            let b = object_string(rt, "the lazy dog, again and again");
            let rope = crate::string::concat(rt, a, b).expect("a rope");
            let r = re(cx, "lazy (\\w+)", "");
            let s: Local<'_> = Local::from_bits(rope);
            let m = cx
                .regexp_exec(r, s.as_string().unwrap(), 0)
                .unwrap()
                .unwrap();
            assert_eq!(spans(&m), "35-43 40-43");
            // Sticky: only at the start index.
            let r = re(cx, "b", "y");
            assert_eq!(exec(cx, r, "abc", 0), "null");
            assert_eq!(exec(cx, r, "abc", 1), "1-2");
            // Unicode mode: a start inside a surrogate pair starts at the pair (V8).
            let pair = [0xd83d, 0xde00];
            let r = re(cx, ".", "u");
            assert_eq!(exec16(cx, r, &pair, 1), "0-2");
            let r = re(cx, ".", "");
            assert_eq!(exec16(cx, r, &pair, 1), "1-2");
        });
    }
}

/// A new string cell (crate API).
fn object_string(rt: &Runtime, s: &str) -> u64 {
    crate::object::new_string(rt, s)
}

#[test]
fn create_errors_and_slots() {
    with_cx(true, |cx, rt| {
        let proto = cx.new_object();
        for (p, f, want) in [
            (
                "a",
                "gg",
                "SyntaxError: Invalid flags supplied to RegExp constructor 'gg'",
            ),
            (
                "a",
                "x",
                "SyntaxError: Invalid flags supplied to RegExp constructor 'x'",
            ),
            (
                "a",
                "uv",
                "SyntaxError: Invalid flags supplied to RegExp constructor 'uv'",
            ),
        ] {
            let (p, f) = (cx.new_string(p).unwrap(), cx.new_string(f).unwrap());
            assert!(cx.regexp_create(proto, p, f).is_err());
            assert_eq!(take_error(cx, rt), want);
        }
        let (p, f) = (cx.new_string("(").unwrap(), cx.new_string("g").unwrap());
        assert!(cx.regexp_create(proto, p, f).is_err());
        let e = take_error(cx, rt);
        assert!(
            e.starts_with("SyntaxError: Invalid regular expression: /(/g: "),
            "{e}"
        );
        let not_re = cx.new_object();
        let s = cx.new_string("a").unwrap();
        assert!(cx.regexp_exec(not_re, s, 0).is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: object is not a RegExp object"
        );
        // The slots: source, flags in the order dgimsuvy, the bits, the program index.
        let (p, f) = (
            cx.new_string("a|b").unwrap(),
            cx.new_string("ysmigd").unwrap(),
        );
        let r = cx.regexp_create(proto, p, f).unwrap();
        assert_eq!(cx.object_class(r), Some(ObjectClass::RegExp));
        assert_eq!(slot_text(cx, r, slot::SOURCE), "a|b");
        assert_eq!(slot_text(cx, r, slot::FLAGS), "dgimsy");
        assert_eq!(slot_text(cx, r, slot::RECORD), "159");
        assert!(
            ProgramTable::index_of(r.bits()).is_some(),
            "compiled at once"
        );
        assert_eq!(cx.get_prototype_of(r).unwrap().bits(), proto.bits());
        let li = cx.get(r, "lastIndex").unwrap();
        assert_eq!(cx.to_rust_string(li).unwrap(), "0");
        let d = cx.get_own_property(r, "lastIndex").unwrap().unwrap();
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(true), Some(false), Some(false))
        );
    });
}

/// Internal slot `i` of the RegExp `r` as Rust text.
fn slot_text<'s>(cx: &mut Cx<'s>, r: Local<'s, JsObject>, i: usize) -> String {
    let v = cx.slot(r, ObjectClass::RegExp, i).unwrap();
    cx.to_rust_string(v).unwrap()
}

/// A script that makes `/a(b)?/g` twice and returns `[first, second]`.
fn two_literals() -> VerifiedUnit {
    let mut s = Asm::script(0);
    let (r0, r1) = (s.b.new_register(), s.b.new_register());
    let k = s.b.string_constant("a(b)?");
    let slot = s.b.feedback_slot(FeedbackSlotKind::Literal);
    s.b.create_reg_exp_literal(k, slot, 0b10).star(r0);
    s.b.create_reg_exp_literal(k, slot, 0b10).star(r1);
    let list = s.b.new_register_list(2);
    s.b.ldar(r0).star(list.get(0)).ldar(r1).star(list.get(1));
    let arr = s.b.feedback_slot(FeedbackSlotKind::Literal);
    s.b.create_empty_array_literal(arr);
    let a = s.b.new_register();
    s.b.star(a);
    for (i, r) in [(0, r0), (1, r1)] {
        let idx = s.b.new_register();
        s.lda_num(f64::from(i)).b.star(idx);
        s.b.ldar(r);
        let st = s.set_keyed;
        s.b.set_keyed_property(a, idx, st);
    }
    s.b.ldar(a).return_();
    unit("", vec![s.finish()])
}

#[test]
fn literal_compiles_at_first_exec() {
    for stress in [false, true] {
        let mut iso = isolate(config(Dispatch::DEFAULT, stress));
        let out = iso.with_cx(|cx| {
            let arr = cx.run_script(two_literals(), "test.js")?;
            let arr = arr.as_object().unwrap();
            let r0 = cx.get(arr, "0")?.as_object().unwrap();
            let r1 = cx.get(arr, "1")?.as_object().unwrap();
            assert!(r0.bits() != r1.bits(), "a new object per evaluation");
            assert_eq!(cx.object_class(r0), Some(ObjectClass::RegExp));
            assert!(ProgramTable::index_of(r0.bits()).is_none(), "lazy");
            let s = cx.new_string("xab").unwrap();
            let m = cx.regexp_exec(r0, s, 0)?.unwrap();
            let i = ProgramTable::index_of(r0.bits()).expect("compiled by the exec");
            let again = cx.regexp_exec(r0, s, 2)?;
            assert_eq!(ProgramTable::index_of(r0.bits()), Some(i), "kept");
            assert!(ProgramTable::index_of(r1.bits()).is_none());
            let flags = cx.slot(r0, ObjectClass::RegExp, slot::FLAGS)?;
            Ok((spans(&m), again.is_none(), cx.to_rust_string(flags)?))
        });
        assert_eq!(out.unwrap(), ("1-3 2-3".into(), true, "g".into()));
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    }
}

// ---- the weak source: a dead RegExp frees its program ------------------------------------------
//
// The RegExps must die, so no copy of their addresses may sit where the conservative scan
// reads (as tests/feedback.rs, W4-P3FIX-DEADSHAPE-TEST): they are made below a 64 KiB pad
// and only their masked addresses come back; the stack is scrubbed before the collection.

const MASK: usize = 0x5a5a_5a5a_5a5a_5a5a;
const DEEP: usize = 64 << 10;
const MADE: usize = 8;

#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

#[inline(never)]
fn below_a_pad<R>(made: impl FnOnce() -> R) -> (R, usize) {
    let mut pad = [0u8; DEEP];
    black_box(&mut pad);
    let r = made();
    (r, black_box(&mut pad).as_ptr().addr())
}

/// Makes [`MADE`] RegExps that nothing keeps and runs each once; returns their masked
/// addresses.
#[inline(never)]
fn made_regexps(iso: &mut Isolate) -> Vec<usize> {
    let mut cx = Cx::new(&mut iso.cx);
    let mut out = Vec::new();
    for i in 0..MADE {
        let r = re(&mut cx, &format!("a{{{i}}}"), "");
        assert!(exec(&mut cx, r, "aaaaaaaaaa", 0).starts_with("0-"));
        out.push(black_box(r.bits() as usize) ^ MASK);
    }
    out
}

#[test]
fn a_dead_regexp_frees_its_program() {
    let mut c = config(Dispatch::DEFAULT, false);
    c.heap.record_conservative_roots = true;
    let mut iso = isolate(c);
    // A RegExp that the test keeps (a Rust local: the conservative scan).
    let kept = {
        let mut cx = Cx::new(&mut iso.cx);
        re(&mut cx, "k+", "").bits()
    };
    let (cells, pad_low) = below_a_pad(|| made_regexps(&mut iso));
    assert_eq!(iso.rt.regexp_programs.live(), MADE + 1);
    scrub_stack();
    iso.collect_garbage();
    let high = ep9_platform::current_thread_stack_bounds().unwrap().high;
    let scan_low = high - 8 * iso.heap().stats().last_stack_words;
    assert!(scan_low > pad_low, "the scan read the frames below the pad");
    let css = iso.rt.heap.last_conservative_roots();
    for &m in &cells {
        let addr = m ^ MASK;
        assert!(
            !css.iter().any(|c| c.addr() == addr),
            "the scan kept {addr:#x}"
        );
    }
    assert_eq!(
        iso.rt.regexp_programs.live(),
        1,
        "only the kept RegExp's entry"
    );
    // The kept one still runs, and a new RegExp reuses a freed index.
    let mut cx = Cx::new(&mut iso.cx);
    let kept: Local<'_> = Local::from_bits(black_box(kept));
    let kept = kept.as_object().unwrap();
    assert_eq!(exec(&mut cx, kept, "xkk", 0), "1-3");
    let fresh = re(&mut cx, "z", "");
    let i = ProgramTable::index_of(fresh.bits()).unwrap();
    assert!((i as usize) < MADE + 1, "index {i} reused");
    assert_eq!(iso.rt.regexp_programs.live(), 2);
    iso.verify_heap().unwrap();
}

#[test]
fn exec_runs_a_pending_interrupt() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let ran = Arc::new(AtomicUsize::new(0));
    let r2 = Arc::clone(&ran);
    iso.interrupt_handle().request(Box::new(move || {
        r2.fetch_add(1, Ordering::SeqCst);
    }));
    let out = iso.with_cx(|cx| {
        let r = re(cx, "b+", "");
        Ok(exec(cx, r, "abbc", 0))
    });
    assert_eq!(out.unwrap(), "1-3");
    assert_eq!(
        ran.load(Ordering::SeqCst),
        1,
        "the callback ran at the exec's poll"
    );
}

#[test]
fn exec_stops_on_termination() {
    let mut iso = isolate(config(Dispatch::DEFAULT, false));
    let out = iso.with_cx(|cx| {
        let r = re(cx, "b+", "");
        let s = cx.new_string("abbc").unwrap();
        Ok(r.bits() != 0 && s.bits() != 0)
    });
    assert!(out.unwrap());
    iso.terminate_handle().terminate();
    let out = iso.with_cx(|cx| {
        let r = re(cx, "b+", "");
        let s = cx.new_string("abbc").unwrap();
        cx.regexp_exec(r, s, 0).map(|m| m.is_some())
    });
    assert!(out.is_err(), "the termination ends the exec: {out:?}");
}

// ---- the receiver stays live through the exec (Codex M4-P2 R1B, high) ----
// The caller's only copy of the RegExp is the argument of `regexp_exec`: it is made in a
// frame below a pad, only its masked bits come back, and the stack is scrubbed before the
// call. Flattening the rope subject then allocates, and under GC stress that collects; in an
// optimized build `exec` must still hold the receiver, or the weak source frees its program
// and the exec panics ("a live RegExp holds a live program index"). Debug builds keep every
// local in a stack slot, so this test fails before the fix only in release
// (`cargo test --release -p ep9_vm --lib tests::regexp`).

/// A RegExp `b+` made below a pad; returns its masked bits only.
#[inline(never)]
fn masked_regexp(cx: &mut Cx<'_>) -> usize {
    let mut pad = [0usize; 8192];
    black_box(&mut pad);
    let r = re(cx, "b+", "");
    let m = black_box(r.bits() as usize) ^ MASK;
    black_box(&mut pad);
    m
}

/// One exec on `s` of the RegExp whose masked bits are `masked`; nothing else holds the
/// RegExp (the bits are unmasked only into the argument).
#[inline(never)]
fn exec_unrooted<'s>(cx: &mut Cx<'s>, masked: usize, s: u64) -> Option<(usize, usize)> {
    let r: Local<'s> = Local::from_bits((black_box(masked) ^ MASK) as u64);
    let s: Local<'s> = Local::from_bits(s);
    let m = cx
        .regexp_exec(r, s.as_string().unwrap(), 0)
        .unwrap()
        .unwrap();
    Some((m.start(), m.end()))
}

#[test]
fn exec_keeps_the_receiver_across_the_flatten() {
    let mut iso = isolate(config(Dispatch::DEFAULT, true));
    let mut cx = Cx::new(&mut iso.cx);
    for i in 0..64 {
        let rt = cx.inner().rt();
        let a = object_string(rt, &"a".repeat(200));
        let b = object_string(rt, &"b".repeat(200));
        let rope = black_box(crate::string::concat(rt, a, b).expect("a rope"));
        let masked = masked_regexp(&mut cx);
        scrub_stack();
        assert_eq!(
            exec_unrooted(&mut cx, masked, rope),
            Some((200, 400)),
            "iteration {i}"
        );
    }
    drop(cx);
    iso.collect_garbage();
    iso.verify_heap().unwrap();
}
