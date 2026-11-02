//! Shape tests (H17): the shapes core of week 4. Every test runs in an isolate with GC
//! stress 1 (a collection before every allocation) and the heap verifier after each
//! collection. A shape reached only through a transition is weak, so the tests keep the
//! shapes they look at again in `ScratchRoots`, never in a `Vec`.

use std::hint::black_box;

use ep9_heap::ScratchRoots;

use super::{config, isolate, on_big_stack};
use crate::Dispatch;
use crate::atoms;
use crate::frame::{NULL, UNDEFINED};
use crate::kinds;
use crate::object::attr;
use crate::runtime::{Root, Runtime, val};
use crate::shape::{self, ACCESSOR, Added, MAX_TRANSITIONS};
use crate::string;

/// Runs `f` with the runtime of a new isolate under GC stress and the verifier, on a big
/// stack. Verifies the heap after `f`.
fn stressed(f: fn(&Runtime)) {
    on_big_stack(move || {
        let mut c = config(Dispatch::DEFAULT, true);
        c.heap.record_conservative_roots = true;
        let mut iso = isolate(c);
        let before = iso.heap().stats().collections;
        f(&iso.rt);
        let n = iso.heap().stats().collections - before;
        // Stress 1 collects before every allocation, so one collection shows that stress is
        // on. A fixed floor breaks when more keys are atoms (H19): a repeated atom key
        // allocates nothing.
        assert!(n > 0, "GC stress is off: no collection ran");
        eprintln!("collections under stress: {n}");
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

/// The property key for `name` (an atom once H19 atoms land).
fn k(rt: &Runtime, name: &str) -> u64 {
    atoms::intern_str(rt, name)
}

fn child(a: Added) -> u64 {
    match a {
        Added::Shape(s) => s,
        Added::Dictionary => panic!("unexpected dictionary mode"),
    }
}

/// `s` plus the data property `name` with default attributes.
fn add(rt: &Runtime, s: u64, name: &str) -> u64 {
    let c = child(shape::add(rt, s, k(rt, name), attr::DEFAULT));
    shape::check(rt, c).unwrap();
    c
}

/// Keeps `v` alive; returns its index in `roots`.
fn keep(roots: &ScratchRoots<'_>, v: u64) -> usize {
    roots.push(val(v))
}

fn kept(roots: &ScratchRoots<'_>, i: usize) -> u64 {
    roots.get(i).to_bits()
}

/// The root shape of plain objects with no prototype and 4 in-object slots.
fn null_root(rt: &Runtime) -> u64 {
    shape::root_shape_for(rt, shape::null_proto_info(rt), kinds::OBJECT, 4)
}

/// The index of `name` in `s`, if `s` has it.
fn index_of(rt: &Runtime, s: u64, name: &str) -> Option<usize> {
    shape::lookup(rt, s, k(rt, name)).map(|f| {
        assert_eq!(usize::from(f.details.field()), f.index);
        f.index
    })
}

/// The text of key `i` of `s`.
fn key_text(s: u64, i: usize) -> String {
    string::str_view(shape::key_at(s, i)).to_string_lossy()
}

// ---- dead cells: the stack must not keep them ----------------------------------------------
//
// A test that expects a shape to die must leave no copy of its address where the conservative
// scan of the next collection reads: the callee-saved registers and `[sp, stack high)` at the
// spill trampoline (ep9_heap css.rs). So the shapes that must die are made, and read, only
// below a 64 KiB pad (W4-P3FIX-DEADSHAPE-TEST, as in tests/ic.rs):
//
//   test frame        gets only masked addresses, and calls nothing that reads the dead
//                     shapes before the collection
//   ---------------   <- the scan stops here (checked: `collect_after_a_pad`)
//   below_a_pad       DEEP bytes of zeros
//   ---------------   <- `pad_low`
//   made_*            frames that make and read the dead shapes (inline(never))
//
// A frame may spill a caller-saved register that it never set, such as the unused payload of a
// returned enum; on x86-64 Linux (debug) that kept a dead shape in tests/ic.rs. Hence every
// read of a dead shape happens below the pad, `transition_count` included (it loads each
// table entry).

const MASK: usize = 0x5a5a_5a5a_5a5a_5a5a;

/// Bytes of zeros between the test's frame and the frames that make the dead shapes: far more
/// than a collection's own frames take (`Heap::collect` down to the spill trampoline).
const DEEP: usize = 64 << 10;

/// Adds `name` to `s` in a frame of its own and drops the child. Run it [`below_a_pad`].
/// Returns the child's address XOR [`MASK`], so no stack word points at it.
#[inline(never)]
fn made_a_dropped_child(rt: &Runtime, s: u64, name: &str) -> usize {
    let c = add(rt, s, name);
    black_box(c as usize) ^ MASK
}

/// Overwrites 256 KiB of stack below the caller, so stale words of returned frames cannot
/// keep a dead cell alive through the conservative scan.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

/// Runs `made` with [`DEEP`] bytes of zeros between its frames and the caller's. `made` must
/// only call an `#[inline(never)]` function that makes the dead shapes and returns their
/// masked addresses. Returns that result and `pad_low`, the lowest byte of the pad: every
/// frame of `made`'s callee lies below it.
#[inline(never)]
fn below_a_pad<R>(made: impl FnOnce() -> R) -> (R, usize) {
    let mut pad = [0u8; DEEP];
    black_box(&mut pad);
    let r = made();
    (r, black_box(&mut pad).as_ptr().addr())
}

/// Scrubs the stack and collects, after the dead shapes were made by [`below_a_pad`] (which
/// gave `pad_low`). Then checks that the collection can show something about them: its
/// conservative scan stopped above the pad, and kept none of `dead` (masked addresses).
/// Inlined (rule 35), so the scrub runs in the test's own frame and its zeros lie under the
/// frame of [`collect_and_check`] too: an out-of-line helper's own frame is not under its
/// own scrub (`tests/ordered_table.rs` `collect`: CI run 40733504045). The collection and
/// the checks stay out of line, so the unmasked addresses they compute never reach the
/// test's frame.
#[inline(always)]
fn collect_after_a_pad(rt: &Runtime, pad_low: usize, dead: &[usize]) {
    scrub_stack();
    collect_and_check(rt, pad_low, dead);
}

/// The collection and the checks of [`collect_after_a_pad`], out of line.
#[inline(never)]
fn collect_and_check(rt: &Runtime, pad_low: usize, dead: &[usize]) {
    rt.heap.collect();
    let high = ep9_platform::current_thread_stack_bounds().unwrap().high;
    let scan_low = high - 8 * rt.heap.stats().last_stack_words;
    assert!(
        scan_low > pad_low,
        "the conservative scan read the frames below the pad ({scan_low:#x} <= {pad_low:#x})"
    );
    assert_not_kept(rt, dead);
}

/// Panics if the last conservative scan kept one of `dead` (masked addresses), which would
/// make the caller's check meaningless.
fn assert_not_kept(rt: &Runtime, dead: &[usize]) {
    let css = rt.heap.last_conservative_roots();
    for &m in dead {
        let addr = m ^ MASK;
        assert!(
            !css.iter().any(|c| c.addr() == addr),
            "the conservative scan kept {addr:#x}"
        );
    }
}

#[test]
fn the_same_add_order_gives_the_same_shape() {
    stressed(|rt| {
        let roots = rt.heap.scratch_roots();
        let r = null_root(rt);
        let a = keep(&roots, add(rt, r, "a"));
        let ab = keep(&roots, add(rt, kept(&roots, a), "b"));
        // Same order again (new key strings with the same text): the same shapes.
        assert_eq!(add(rt, r, "a"), kept(&roots, a));
        assert_eq!(add(rt, kept(&roots, a), "b"), kept(&roots, ab));
        // {b, a} differs from {a, b}.
        let b = keep(&roots, add(rt, r, "b"));
        let ba = keep(&roots, add(rt, kept(&roots, b), "a"));
        let (ab, ba) = (kept(&roots, ab), kept(&roots, ba));
        assert_ne!(ab, ba);
        assert_eq!((shape::count(ab), shape::count(ba)), (2, 2));
        assert_eq!((key_text(ab, 0), key_text(ab, 1)), ("a".into(), "b".into()));
        assert_eq!((key_text(ba, 0), key_text(ba, 1)), ("b".into(), "a".into()));
        assert_eq!(index_of(rt, ab, "a"), Some(0));
        assert_eq!(index_of(rt, ba, "a"), Some(1));
        // Other attributes give another transition.
        let hidden = child(shape::add(rt, r, k(rt, "a"), attr::HIDDEN));
        assert_ne!(hidden, kept(&roots, a));
        assert_eq!(shape::details_at(hidden, 0).attrs(), attr::HIDDEN);
        assert_eq!(shape::transition_count(r), 3);
        // Every shape keeps the prototype, kind and capacity of its root.
        for s in [ab, ba, hidden] {
            assert_eq!(shape::proto(s), NULL);
            assert_eq!(shape::object_kind(s), kinds::OBJECT.get());
            assert_eq!(shape::capacity(s), 4);
            assert!(shape::is_extensible(s) && !shape::is_dictionary(s));
        }
    });
}

#[test]
fn a_chain_shares_one_descriptor_array_and_a_branch_copies_the_prefix() {
    stressed(|rt| {
        let roots = rt.heap.scratch_roots();
        let r = null_root(rt);
        assert_eq!(shape::descriptors(r), UNDEFINED);
        let s1 = keep(&roots, add(rt, r, "a"));
        let s2 = keep(&roots, add(rt, kept(&roots, s1), "b"));
        let s3 = keep(&roots, add(rt, kept(&roots, s2), "c"));
        let d = shape::descriptors(kept(&roots, s1));
        assert_eq!(kinds::cell_kind(d), kinds::DESCRIPTORS.get());
        assert_eq!(shape::descriptors(kept(&roots, s2)), d);
        assert_eq!(shape::descriptors(kept(&roots, s3)), d);
        // The prefix rule: {a} does not see b, although its array holds it.
        assert_eq!(index_of(rt, kept(&roots, s1), "b"), None);
        assert_eq!(index_of(rt, kept(&roots, s2), "b"), Some(1));
        // A branch from {a} copies the prefix [a] and appends x.
        let x = keep(&roots, add(rt, kept(&roots, s1), "x"));
        let dx = shape::descriptors(kept(&roots, x));
        assert_ne!(dx, d);
        assert_eq!(key_text(kept(&roots, x), 0), "a");
        assert_eq!(key_text(kept(&roots, x), 1), "x");
        assert_eq!(index_of(rt, kept(&roots, x), "b"), None);
        assert_eq!(index_of(rt, kept(&roots, s3), "x"), None);
        assert_eq!(shape::descriptors(kept(&roots, s1)), d);
        // The owner {a,b,c} appends d in place (capacity 4), then e grows the array: every
        // shape of the chain moves to the new array, the branch keeps its own.
        let s4 = keep(&roots, add(rt, kept(&roots, s3), "d"));
        assert_eq!(shape::descriptors(kept(&roots, s4)), d);
        let s5 = keep(&roots, add(rt, kept(&roots, s4), "e"));
        let d5 = shape::descriptors(kept(&roots, s5));
        assert_ne!(d5, d);
        for s in [s1, s2, s3, s4] {
            assert_eq!(shape::descriptors(kept(&roots, s)), d5);
        }
        assert_eq!(shape::descriptors(kept(&roots, x)), dx);
        // A non-owner in the middle of the chain branches by copying: {a,b} + y.
        let y = keep(&roots, add(rt, kept(&roots, s2), "y"));
        assert_ne!(shape::descriptors(kept(&roots, y)), d5);
        assert_eq!(index_of(rt, kept(&roots, y), "c"), None);
        assert_eq!(index_of(rt, kept(&roots, y), "y"), Some(2));
        for s in [s1, s2, s3, s4, s5, x, y] {
            shape::check(rt, kept(&roots, s)).unwrap();
        }
    });
}

#[test]
fn lookups_hit_and_miss_at_8_and_at_9_and_more_keys() {
    stressed(|rt| {
        const N: usize = 12;
        let names: Vec<String> = (0..N).map(|i| format!("key{i}")).collect();
        let roots = rt.heap.scratch_roots();
        let mut s = keep(&roots, null_root(rt));
        let first = s;
        for name in &names {
            s = keep(&roots, add(rt, kept(&roots, s), name));
        }
        // Shape i (count i) sees exactly keys 0 .. i, at their own index, whether it scans
        // (count <= 8) or uses the hash index (count > 8). All share one array of 12.
        let d = shape::descriptors(kept(&roots, s));
        for (i, si) in (first..=s).enumerate() {
            let shape_i = kept(&roots, si);
            assert_eq!(shape::count(shape_i), i);
            if i > 0 {
                assert_eq!(shape::descriptors(shape_i), d);
            }
            for (j, name) in names.iter().enumerate() {
                let want = (j < i).then_some(j);
                assert_eq!(index_of(rt, shape_i, name), want, "shape {i}, key {j}");
            }
            assert_eq!(index_of(rt, shape_i, "missing"), None);
            assert_eq!(index_of(rt, shape_i, ""), None);
        }
        // A probe that is not an atom (a fresh string) finds the key by content, through the
        // scan and through the index.
        let probe = string::new_string(rt, "key7");
        assert_eq!(
            shape::lookup(rt, kept(&roots, first + 8), probe).map(|f| f.index),
            Some(7)
        );
        assert_eq!(
            shape::lookup(rt, kept(&roots, s), probe).map(|f| f.index),
            Some(7)
        );
        assert_eq!(shape::lookup(rt, kept(&roots, first + 7), probe), None);
        // Those content compares were on purpose; the EP9_ASSERT_ATOM_KEYS gate checks the rest.
        rt.atoms.borrow_mut().reset_non_atom_compares();
        // A branch at 10 keys copies a prefix longer than 8 and indexes it.
        let b = add(rt, kept(&roots, first + 10), "branch");
        assert_ne!(shape::descriptors(b), d);
        assert_eq!(index_of(rt, b, "key9"), Some(9));
        assert_eq!(index_of(rt, b, "key10"), None);
        assert_eq!(index_of(rt, b, "branch"), Some(10));
    });
}

#[test]
fn a_dead_branch_is_cleared_after_a_gc() {
    stressed(|rt| {
        let roots = rt.heap.scratch_roots();
        let r = null_root(rt);
        let a = keep(&roots, add(rt, r, "a"));
        // The single-transition slot: {a} -> {a,b} dies.
        let s = kept(&roots, a);
        let (dead, pad_low) = below_a_pad(|| made_a_dropped_child(rt, s, "b"));
        collect_after_a_pad(rt, pad_low, &[dead]);
        assert_eq!(shape::transition_count(kept(&roots, a)), 0);
        assert!(rt.heap.stats().last_weak_cleared >= 1);
        // Adding b again makes a new child, which shares {a}'s array again: the dead
        // child's entry is the same key and attributes.
        let ab = keep(&roots, add(rt, kept(&roots, a), "b"));
        assert_eq!(
            shape::descriptors(kept(&roots, ab)),
            shape::descriptors(kept(&roots, a))
        );
        assert_eq!(shape::transition_count(kept(&roots, a)), 1);
        // The transition table: 6 branches from {a,b}, the even ones kept.
        let s = kept(&roots, ab);
        let (dead, pad_low) = below_a_pad(|| made_branches(rt, &roots, s));
        collect_after_a_pad(rt, pad_low, &dead);
        assert_eq!(shape::transition_count(kept(&roots, ab)), 3);
        for i in [0, 2, 4] {
            let name = format!("x{i}");
            let again = add(rt, kept(&roots, ab), &name);
            assert_eq!(again, kept(&roots, roots.len() - 3 + i / 2), "{name}");
        }
        assert_eq!(shape::transition_count(kept(&roots, ab)), 3);
        // A new branch reuses a cleared entry.
        let x9 = add(rt, kept(&roots, ab), "x9");
        assert_eq!(shape::transition_count(kept(&roots, ab)), 4);
        shape::check(rt, x9).unwrap();
    });
}

/// Adds x0 .. x5 to `s`, keeps x0, x2 and x4 (in that order, at the end of `roots`) and
/// returns the masked addresses of the others. Run it [`below_a_pad`].
#[inline(never)]
fn made_branches(rt: &Runtime, roots: &ScratchRoots<'_>, s: u64) -> Vec<usize> {
    let mut dead = Vec::new();
    for i in 0..6 {
        let c = add(rt, s, &format!("x{i}"));
        if i % 2 == 0 {
            keep(roots, c);
        } else {
            dead.push(black_box(c as usize) ^ MASK);
        }
    }
    // Under GC stress the dropped ones may die before this returns. The count reads every
    // table entry, so it runs here, below the pad.
    assert!(shape::transition_count(s) >= 3);
    dead
}

#[test]
fn the_65th_transition_reports_dictionary() {
    stressed(|rt| {
        let roots = rt.heap.scratch_roots();
        let base = keep(&roots, add(rt, null_root(rt), "base"));
        for i in 0..MAX_TRANSITIONS {
            let c = add(rt, kept(&roots, base), &format!("t{i}"));
            keep(&roots, c);
        }
        assert_eq!(shape::transition_count(kept(&roots, base)), 64);
        assert_eq!(
            shape::add(rt, kept(&roots, base), k(rt, "t64"), attr::DEFAULT),
            Added::Dictionary
        );
        // An existing transition still works.
        assert_eq!(
            shape::add(rt, kept(&roots, base), k(rt, "t7"), attr::DEFAULT),
            Added::Shape(kept(&roots, base + 1 + 7))
        );
        // So does a property added further down the tree.
        let deeper = add(rt, kept(&roots, base + 1), "z");
        assert_eq!(shape::count(deeper), 3);
    });
}

#[test]
fn remove_last_returns_the_parent() {
    stressed(|rt| {
        let roots = rt.heap.scratch_roots();
        let r = keep(&roots, null_root(rt));
        let s1 = keep(&roots, add(rt, kept(&roots, r), "a"));
        let s2 = keep(&roots, add(rt, kept(&roots, s1), "b"));
        let s3 = add(rt, kept(&roots, s2), "c");
        assert_eq!(shape::remove_last(s3), kept(&roots, s2));
        assert_eq!(shape::remove_last(kept(&roots, s2)), kept(&roots, s1));
        assert_eq!(shape::remove_last(kept(&roots, s1)), kept(&roots, r));
        assert_eq!(shape::parent(kept(&roots, r)), UNDEFINED);
        // Adding c again gives the same {a,b,c}.
        assert_eq!(add(rt, kept(&roots, s2), "c"), s3);
    });
}

#[test]
fn root_and_dictionary_shapes_are_kept_per_prototype() {
    stressed(|rt| {
        let null_info = shape::null_proto_info(rt);
        assert_eq!(kinds::cell_kind(null_info), kinds::PROTO_INFO.get());
        let before = shape::proto_info_len(null_info);
        let r4 = null_root(rt);
        // The same (kind, capacity) gives the same root, after collections too.
        rt.heap.collect();
        assert_eq!(null_root(rt), r4);
        // (Capacity 6: the realm made the (OBJECT, 8) root for `Object.prototype` already.)
        let r6 = shape::root_shape_for(rt, null_info, kinds::OBJECT, 6);
        let f4 = shape::root_shape_for(rt, null_info, kinds::FUNCTION, 4);
        assert!(r4 != r6 && r4 != f4 && r6 != f4);
        assert_eq!(shape::capacity(r6), 6);
        assert_eq!(shape::object_kind(f4), kinds::FUNCTION.get());
        for s in [r4, r6, f4] {
            assert_eq!((shape::count(s), shape::proto(s)), (0, NULL));
            shape::check(rt, s).unwrap();
        }
        // Dictionary shapes: one per (kind, extensible).
        let dx = shape::dictionary_shape_for(rt, null_info, kinds::OBJECT, true);
        let dn = shape::dictionary_shape_for(rt, null_info, kinds::OBJECT, false);
        assert_ne!(dx, dn);
        assert_eq!(
            shape::dictionary_shape_for(rt, null_info, kinds::OBJECT, true),
            dx
        );
        assert!(shape::is_dictionary(dx) && shape::is_extensible(dx));
        assert!(shape::is_dictionary(dn) && !shape::is_extensible(dn));
        assert!(!shape::is_dictionary(r4));
        assert_eq!(
            shape::add(rt, dx, k(rt, "a"), attr::DEFAULT),
            Added::Dictionary
        );
        shape::check(rt, dn).unwrap();
        assert_eq!(shape::proto_info_len(null_info), before + 5);
        // A prototype object gets its own PROTO_INFO; its roots carry it as [[Prototype]].
        let roots = rt.heap.scratch_roots();
        let proto = rt.root(Root::ObjectPrototype);
        let info = keep(&roots, shape::new_proto_info(rt, proto));
        let p4 = shape::root_shape_for(rt, kept(&roots, info), kinds::OBJECT, 4);
        assert_ne!(p4, r4);
        assert_eq!(shape::proto(p4), proto);
        assert_eq!(
            shape::root_shape_for(rt, kept(&roots, info), kinds::OBJECT, 4),
            p4
        );
        let pd = shape::dictionary_shape_for(rt, kept(&roots, info), kinds::ARRAY, true);
        assert_eq!(shape::proto(pd), proto);
        assert_eq!(shape::object_kind(pd), kinds::ARRAY.get());
        assert_eq!(shape::proto_info_len(kept(&roots, info)), 2);
        // A chain from that root keeps the prototype.
        let pa = add(rt, p4, "a");
        assert_eq!(shape::proto(pa), proto);
    });
}

#[test]
fn index_keys_and_limits_go_to_dictionary_mode() {
    stressed(|rt| {
        let r = null_root(rt);
        for name in ["0", "1", "4294967294"] {
            assert_eq!(
                shape::add(rt, r, k(rt, name), attr::DEFAULT),
                Added::Dictionary,
                "{name}"
            );
        }
        // Not array indices: these are ordinary keys.
        for name in ["01", "-1", "4294967295", "1.5"] {
            let c = add(rt, r, name);
            assert_eq!(index_of(rt, c, name), Some(0));
        }
    });
}

#[test]
fn accessor_and_attribute_bits_round_trip() {
    stressed(|rt| {
        let r = null_root(rt);
        let g = child(shape::add(rt, r, k(rt, "g"), ACCESSOR | attr::CONFIGURABLE));
        let d = shape::details_at(g, 0);
        assert!(d.is_accessor());
        assert_eq!(d.attrs(), ACCESSOR | attr::CONFIGURABLE);
        assert_eq!(d.field(), 0);
        let f = shape::lookup(rt, g, k(rt, "g")).unwrap();
        assert_eq!((f.index, f.details), (0, d));
        // The same key as data is another transition.
        let data = add(rt, r, "g");
        assert_ne!(data, g);
        assert!(!shape::details_at(data, 0).is_accessor());
    });
}

#[test]
fn the_fast_property_limit_is_1020() {
    stressed(|rt| {
        let roots = rt.heap.scratch_roots();
        let i = keep(&roots, null_root(rt));
        for n in 0..shape::MAX_FAST_PROPERTIES {
            let next = child(shape::add(
                rt,
                kept(&roots, i),
                k(rt, &format!("p{n}")),
                attr::DEFAULT,
            ));
            roots.set(i, val(next));
        }
        let last = kept(&roots, i);
        assert_eq!(shape::count(last), 1020);
        assert_eq!(
            shape::add(rt, last, k(rt, "p1020"), attr::DEFAULT),
            Added::Dictionary
        );
        for n in [0, 8, 9, 511, 1019] {
            assert_eq!(index_of(rt, last, &format!("p{n}")), Some(n));
        }
        assert_eq!(index_of(rt, last, "p1020"), None);
        shape::check(rt, last).unwrap();
    });
}
