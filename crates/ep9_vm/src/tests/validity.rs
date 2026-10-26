//! Tests of validity cells (`validity.rs`, VALIDITY_CELL kind 79; M4-P1 VALIDITY, H16 K5):
//! a change below a cached holder, changes that must not miss, realms, the GC of prototypes
//! and of users, no counter that saturates, the depth limit, and a random walk that checks the
//! invariant and soundness (a valid cell's chain has not changed).
//!
//! The IC tests run under every dispatch expansion, with and without GC stress, with eager
//! feedback ([`each`], the `tests::ic` harness).

use std::hint::black_box;

use ep9_heap::GlobalHandle;

use super::ic::{
    ADD, MASK, MISSING, PROTO_LOAD, Site, T, below_a_pad, collect_after_a_pad, each, int, key,
    object_prototype, object_with, own,
};
use crate::builtins::IntrinsicId;
use crate::frame::{NULL, UNDEFINED};
use crate::ic::State;
use crate::object::{self, attr};
use crate::property;
use crate::props;
use crate::realm;
use crate::runtime::Runtime;
use crate::shape;
use crate::validity::{self, MAX_DEPTH};

/// The current validity cell of the prototype `p` (undefined if it has none yet).
fn cell_of(rt: &Runtime, p: u64) -> u64 {
    shape::validity_cell_of(shape::proto_info_of(rt, p))
}

/// The +16 word (the validity cell) of the case `f<i>` keeps for the shape of `o`.
fn case_cell(t: &T, i: usize, o: u64) -> u64 {
    t.describe(i, shape::shape_of(o)).expect("a case").1
}

/// Adds the data property `name` = `v` {w, e, c} to `o`: a structural change.
fn add(rt: &Runtime, o: u64, name: &str, v: u64) {
    props::own_define(rt, o, key(rt, name), v, attr::DEFAULT);
}

#[test]
fn a_change_below_a_cached_holder_misses() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("m"), Site::Get("nope")]);
        let op = object_prototype(t.rt());
        let p2 = object_with(t.rt(), op, &[("m", int(1))]);
        let p1 = object::new_object(t.rt(), p2);
        let o = object::new_object(t.rt(), p1);
        assert_eq!(t.ok(0, o, UNDEFINED), int(1));
        let (kind, c1, holder, _) = t.describe(0, shape::shape_of(o)).unwrap();
        assert_eq!((kind, holder), (PROTO_LOAD, p2));
        // The case keeps the cell of the receiver shape's prototype, `p1`; the cells of the
        // whole chain are valid, one deeper each, and each is a user of the one above.
        assert_eq!(c1, cell_of(t.rt(), p1));
        let (c2, cop) = (cell_of(t.rt(), p2), cell_of(t.rt(), op));
        let cnull = cell_of(t.rt(), NULL);
        for (cell, depth) in [(cnull, 0), (cop, 1), (c2, 2), (c1, 3)] {
            assert!(validity::is_valid(cell));
            assert_eq!(validity::depth(cell), depth);
        }
        assert!(validity::has_user(c2, c1));
        assert!(validity::has_user(cop, c2));
        // `p1` (below the holder) gets its own `m`: its cell goes, the ones above stay.
        add(t.rt(), p1, "m", int(2));
        assert!(!validity::is_valid(c1));
        assert!(validity::is_valid(c2) && validity::is_valid(cop));
        assert_eq!(
            validity::user_count(c1),
            0,
            "an invalid cell drops its users"
        );
        assert_eq!(t.ok(0, o, UNDEFINED), int(2));
        let (kind, c1b, holder, _) = t.describe(0, shape::shape_of(o)).unwrap();
        assert_eq!((kind, holder), (PROTO_LOAD, p1));
        assert!(c1b != c1 && validity::is_valid(c1b));
        // A change above the holder clears every cell below it.
        add(t.rt(), op, "icZz", int(0));
        for cell in [cop, c2, c1b] {
            assert!(!validity::is_valid(cell));
        }
        assert!(
            validity::is_valid(cnull),
            "the null prototype's cell never goes"
        );
        assert_eq!(t.ok(0, o, UNDEFINED), int(2));
        // A missing property under the same chain, then the holder gets it.
        assert_eq!(t.ok(1, o, UNDEFINED), UNDEFINED);
        assert_eq!(t.kind(1, shape::shape_of(o)), Some(MISSING));
        add(t.rt(), p2, "nope", int(5));
        assert_eq!(t.ok(1, o, UNDEFINED), int(5));
        black_box((p1, p2));
    });
}

#[test]
fn changes_off_the_chain_and_value_stores_do_not_miss() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("m"), Site::Set("m", true)]);
        let op = object_prototype(t.rt());
        let p = object_with(t.rt(), op, &[("m", int(1))]);
        let o = object::new_object(t.rt(), p);
        assert_eq!(t.ok(0, o, UNDEFINED), int(1));
        let h = t.handler(0, shape::shape_of(o)).unwrap();
        // Another prototype under Object.prototype, and a prototype below `p` that `o` does
        // not use: structural changes of both, and a value store to `p`. The epoch moves.
        let q = object_with(t.rt(), op, &[("a", int(0))]);
        let _qchild = object::new_object(t.rt(), q);
        let r = object::new_object(t.rt(), p);
        let _rchild = object::new_object(t.rt(), r);
        let (cq, cr) = (
            validity::cell_for(t.rt(), q).unwrap(),
            validity::cell_for(t.rt(), r).unwrap(),
        );
        assert!(validity::has_user(cell_of(t.rt(), p), cr));
        let e = t.epoch();
        add(t.rt(), q, "b", int(0));
        add(t.rt(), r, "m", int(9));
        assert!(t.epoch() > e);
        assert!(!validity::is_valid(cq) && !validity::is_valid(cr));
        t.ok(1, p, int(3));
        assert_eq!(t.ok(0, o, UNDEFINED), int(3));
        assert_eq!(
            t.handler(0, shape::shape_of(o)),
            Some(h),
            "a hit: the case was not made again"
        );
        assert!(validity::is_valid(case_cell(&t, 0, o)));
        black_box((q, r));
    });
}

#[test]
fn each_realm_keeps_its_own_cells() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("m"), Site::Get("n")]);
        let op_a = object_prototype(t.rt());
        let realm_b = realm::create(&mut t.iso.cx).unwrap();
        let op_b = realm::intrinsic_in(t.rt(), realm_b, IntrinsicId::ObjectPrototype);
        assert!(op_b != op_a && object::is_object(op_b));
        let kept = [t.keep(realm_b), t.keep(op_b)];
        let pa = object_with(t.rt(), op_a, &[("m", int(1))]);
        let oa = object::new_object(t.rt(), pa);
        let pb = object_with(t.rt(), op_b, &[("n", int(2))]);
        let ob = object::new_object(t.rt(), pb);
        assert_eq!(t.ok(0, oa, UNDEFINED), int(1));
        assert_eq!(t.ok(1, ob, UNDEFINED), int(2));
        let (ca, cb) = (case_cell(&t, 0, oa), case_cell(&t, 1, ob));
        // A change of one realm's Object.prototype clears only that realm's chains.
        add(t.rt(), op_b, "icB", int(0));
        assert!(validity::is_valid(ca) && !validity::is_valid(cb));
        add(t.rt(), op_a, "icA", int(0));
        assert!(!validity::is_valid(ca));
        assert_eq!(t.ok(0, oa, UNDEFINED), int(1));
        assert_eq!(t.ok(1, ob, UNDEFINED), int(2));
        assert!(validity::is_valid(case_cell(&t, 0, oa)));
        assert!(validity::is_valid(case_cell(&t, 1, ob)));
        // Each realm's Object.prototype cell hangs off the null prototype's cell.
        let cnull = cell_of(t.rt(), NULL);
        assert!(validity::has_user(cnull, cell_of(t.rt(), op_a)));
        assert!(validity::has_user(cnull, cell_of(t.rt(), op_b)));
        for h in kept {
            t.release(h);
        }
    });
}

/// Makes `n` prototypes below `p` and a cell for each (users of `p`'s cell), none of which
/// outlives the call. Run it [`below_a_pad`]. Returns the masked addresses of the prototypes.
#[inline(never)]
fn made_lone_users(t: &mut T, p: u64, n: usize) -> Vec<usize> {
    let mut dead = Vec::new();
    for _ in 0..n {
        let q = object::new_object(t.rt(), p);
        let _child = object::new_object(t.rt(), q);
        let c = validity::cell_for(t.rt(), q).unwrap();
        assert!(validity::is_valid(c));
        dead.push(black_box(q as usize) ^ MASK);
    }
    dead
}

#[test]
fn users_that_die_leave_the_list() {
    each(|c| {
        let mut t = T::new(c, &[]);
        let op = object_prototype(t.rt());
        let p = object_with(t.rt(), op, &[("a", int(0))]);
        let kept = t.keep(p);
        let _child = object::new_object(t.rt(), p);
        let cp = validity::cell_for(t.rt(), p).unwrap();
        let (dead, pad_low) = below_a_pad(|| made_lone_users(&mut t, p, 20));
        collect_after_a_pad(&mut t, pad_low, &dead);
        assert!(validity::is_valid(cp));
        assert_eq!(validity::user_count(cp), 0, "dead users are cleared");
        // A new user is the only live entry.
        let q = object::new_object(t.rt(), p);
        let _child = object::new_object(t.rt(), q);
        let cq = validity::cell_for(t.rt(), q).unwrap();
        assert_eq!(validity::user_count(cp), 1);
        assert!(validity::has_user(cp, cq));
        add(t.rt(), p, "b", int(1));
        assert!(!validity::is_valid(cp) && !validity::is_valid(cq));
        t.collect();
        t.release(kept);
    });
}

/// Builds `o → q → p → Object.prototype` and loads the missing `o.x` through `f0`: the case
/// keeps `q`'s cell, a user of `p`'s. Nothing but the IC keeps any of them after the call.
/// Run it [`below_a_pad`]. Returns the masked addresses of `o`, `q` and `p`.
#[inline(never)]
fn made_a_case_on_lone_prototypes(t: &mut T) -> [usize; 3] {
    let op = object_prototype(t.rt());
    let p = object_with(t.rt(), op, &[("a", int(0))]);
    let q = object::new_object(t.rt(), p);
    let o = object::new_object(t.rt(), q);
    assert_eq!(t.ok(0, o, UNDEFINED), UNDEFINED);
    let (kind, cq, _, _) = t.describe(0, shape::shape_of(o)).unwrap();
    assert_eq!(kind, MISSING);
    assert_eq!(cq, cell_of(t.rt(), q));
    assert!(validity::has_user(cell_of(t.rt(), p), cq));
    [o, q, p].map(|x| black_box(x as usize) ^ MASK)
}

#[test]
fn a_prototype_dies_while_a_handler_keeps_its_users_cell() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("x")]);
        let (dead, pad_low) = below_a_pad(|| made_a_case_on_lone_prototypes(&mut t));
        collect_after_a_pad(&mut t, pad_low, &dead);
        // The cell keeps no prototype: `o`'s shape (the case key) died with `q` and `p`, while
        // the handler (the strong value half) and its cell live on.
        assert_eq!(t.state(0), State::Uninitialized);
        let handler = crate::feedback::value(t.vector(0), super::ic::SLOT);
        let (kind, cell, _, _) = crate::ic::describe_handler(handler);
        assert_eq!(kind, MISSING);
        assert!(validity::is_valid(cell));
        // The site works on.
        let op = object_prototype(t.rt());
        let o2 = object_with(t.rt(), op, &[("x", int(4))]);
        assert_eq!(t.ok(0, o2, UNDEFINED), int(4));
        t.collect();
    });
}

#[test]
fn two_to_the_32_changes_saturate_nothing() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("m"), Site::Set("k", false)]);
        let op = object_prototype(t.rt());
        let p = object_with(t.rt(), op, &[("m", int(1))]);
        let kept = t.keep(p);
        let o = object::new_object(t.rt(), p);
        // Two changes short of the epoch's maximum: the loop below saturates it.
        t.rt().shapes.proto_epoch.set(u32::MAX - 2);
        for round in 0..60 {
            let name = if round % 2 == 0 { "icT" } else { "icU" };
            add(t.rt(), p, name, int(round));
            let s = key(t.rt(), name);
            assert!(property::delete(&mut t.iso.cx, p, s, true).unwrap());
            // Each round clears the cell; the next load makes a case under a new one.
            assert_eq!(t.ok(0, o, UNDEFINED), int(1));
            let (kind, cell, _, _) = t.describe(0, shape::shape_of(o)).unwrap();
            assert_eq!(kind, PROTO_LOAD);
            assert!(validity::is_valid(cell));
            assert_eq!(validity::depth(cell), 2);
            let h = t.handler(0, shape::shape_of(o));
            assert_eq!(t.ok(0, o, UNDEFINED), int(1));
            assert_eq!(t.handler(0, shape::shape_of(o)), h, "a hit");
            // An add under the saturated epoch is a real case too.
            let x = object::new_object(t.rt(), p);
            t.ok(1, x, int(round));
            assert_eq!(own(t.rt(), x, "k"), Some(int(round)));
        }
        assert_eq!(t.epoch(), u32::MAX);
        let root = shape::shape_of(object::new_object(t.rt(), p));
        assert_eq!(t.kind(1, root), Some(ADD));
        assert!(validity::is_valid(t.describe(1, root).unwrap().1));
        t.release(kept);
    });
    // Many invalidations of one chain, with no IC: nothing in a cell counts.
    each(|c| {
        let t = T::new(c, &[]);
        let rt = t.rt();
        let op = object_prototype(rt);
        let p = object_with(rt, op, &[("m", int(1))]);
        let q = object::new_object(rt, p);
        let _child = object::new_object(rt, q);
        for i in 0..2_000 {
            let cq = validity::cell_for(rt, q).unwrap();
            assert_eq!(validity::depth(cq), 3);
            validity::invalidate(rt, cell_of(rt, if i % 2 == 0 { p } else { q }));
            assert!(!validity::is_valid(cq));
        }
        black_box((p, q));
    });
}

#[test]
fn a_chain_deeper_than_max_depth_gets_no_cell() {
    each(|c| {
        let mut t = T::new(c, &[Site::Get("m"), Site::Get("m")]);
        let op = object_prototype(t.rt());
        // `top` has depth 2 (null 0, Object.prototype 1); each object below one more.
        let top = object_with(t.rt(), op, &[("m", int(7))]);
        let mut chain = vec![t.keep(top)];
        let mut p = top;
        let mut depth = 2;
        while depth < MAX_DEPTH {
            p = object::new_object(t.rt(), p);
            chain.push(t.keep(p));
            depth += 1;
        }
        // `p` has depth MAX_DEPTH: an object on it gets a case.
        let o = object::new_object(t.rt(), p);
        assert_eq!(t.ok(0, o, UNDEFINED), int(7));
        assert_eq!(t.kind(0, shape::shape_of(o)), Some(PROTO_LOAD));
        assert_eq!(validity::depth(case_cell(&t, 0, o)), MAX_DEPTH);
        // One deeper: no cell, no case; the generic path answers.
        let deeper = object::new_object(t.rt(), p);
        let o2 = object::new_object(t.rt(), deeper);
        assert_eq!(validity::cell_for(t.rt(), deeper), None);
        for _ in 0..3 {
            assert_eq!(t.ok(1, o2, UNDEFINED), int(7));
        }
        assert_eq!(t.state(1), State::Uninitialized);
        add(t.rt(), top, "m2", int(0));
        assert_eq!(t.ok(1, o2, UNDEFINED), int(7));
        for h in chain {
            t.release(h);
        }
        black_box((o, o2));
    });
}

/// One cell the random walk saw valid: the cell, its prototype, and the (object, shape) pairs
/// of its chain then. Rooted by handles.
struct Seen {
    cell: u64,
    chain: Vec<(u64, u64)>,
    handles: Vec<GlobalHandle>,
}

/// The chain of `p` as (object, shape) pairs, to null.
fn chain_of(mut p: u64) -> Vec<(u64, u64)> {
    let mut v = Vec::new();
    while p != NULL {
        let s = shape::shape_of(p);
        v.push((p, s));
        p = shape::proto(s);
    }
    v
}

/// A random walk over 8 prototypes (a forest under Object.prototype): adds, deletes, value
/// stores, `[[SetPrototypeOf]]`, dictionary mode and `cell_for`. After each step: every
/// valid cell seen so far guards a chain whose objects kept their shapes (soundness), every
/// valid current cell's chain has valid current cells (the invariant), and the null
/// prototype's cell is valid.
#[test]
fn a_random_walk_keeps_the_invariant_and_soundness() {
    each(|c| {
        let mut t = T::new(c, &[]);
        let op = object_prototype(t.rt());
        let mut handles = Vec::new();
        let mut pool = Vec::new();
        for i in 0..8 {
            let up = if i < 3 { op } else { pool[i % 3] };
            let p = object_with(t.rt(), up, &[("a", int(i as i32))]);
            let _child = object::new_object(t.rt(), p);
            handles.push(t.keep(p));
            pool.push(p);
        }
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = |n: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % n
        };
        let mut seen: Vec<Seen> = Vec::new();
        for step in 0..400 {
            let p = pool[next(8) as usize];
            match next(7) {
                0 => add(t.rt(), p, &format!("k{}", next(4)), int(step)),
                1 => {
                    let k = key(t.rt(), &format!("k{}", next(4)));
                    property::delete(&mut t.iso.cx, p, k, true).unwrap();
                }
                2 => props::own_define(t.rt(), p, key(t.rt(), "a"), int(step), attr::DEFAULT),
                3 => {
                    let to = if next(3) == 0 {
                        op
                    } else {
                        pool[next(8) as usize]
                    };
                    property::set_prototype_of(t.rt(), p, to);
                }
                4 if next(8) == 0 => crate::props::to_dictionary(t.rt(), p),
                _ => {
                    if let Some(cell) = validity::cell_for(t.rt(), p) {
                        let chain = chain_of(p);
                        let mut hs = vec![t.keep(cell)];
                        hs.extend(chain.iter().map(|&(_, s)| t.keep(s)));
                        seen.push(Seen {
                            cell,
                            chain,
                            handles: hs,
                        });
                    }
                }
            }
            for s in &seen {
                if validity::is_valid(s.cell) {
                    for &(o, sh) in &s.chain {
                        assert_eq!(
                            shape::shape_of(o),
                            sh,
                            "step {step}: a valid cell over a changed chain"
                        );
                    }
                }
            }
            for &p in &pool {
                if validity::is_valid(cell_of(t.rt(), p)) {
                    for (o, _) in chain_of(p) {
                        assert!(
                            validity::is_valid(cell_of(t.rt(), o)),
                            "step {step}: a valid cell below an invalid one"
                        );
                    }
                }
            }
            assert!(validity::is_valid(cell_of(t.rt(), NULL)));
            if step % 100 == 99 {
                t.collect();
            }
        }
        assert!(seen.len() > 20, "the walk made cells ({})", seen.len());
        for s in seen {
            for h in s.handles {
                t.release(h);
            }
        }
        for h in handles {
            t.release(h);
        }
    });
}
