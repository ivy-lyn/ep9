//! OBJECT_LAYOUT 4 and IC_LAYOUT 2 (M4-P0, owner P0-VM4; plan v9 §2.3, §2.5 task 8): the
//! versions, the new kind ids (declared, not registered), `is_callable` on PROXY_CALLABLE, the
//! IntrinsicIds 64-71 and ObjectClass ids 11-13, the IC_LAYOUT 2 constants, and the M4-P0
//! Runtime fields (the Math.random state, the kept-objects root slot).

use std::hint::black_box;

use ep9_bytecode::FeedbackSlotKind;
use ep9_heap::{CellKind, CellSize, KindId};

use super::{config, isolate};
use crate::abi::ObjectClass;
use crate::builtins::{self, IntrinsicId};
use crate::frame::UNDEFINED;
use crate::runtime::val;
use crate::{Cx, Dispatch, Isolate, ic, kinds, object, versions};

/// Runs `f` with a builtin-ABI context of a fresh isolate.
fn with_cx<R>(stress: bool, f: impl FnOnce(&mut Cx<'_>) -> R) -> R {
    let mut iso = isolate(config(Dispatch::DEFAULT, stress));
    let iso: &mut Isolate = &mut iso;
    let mut cx = Cx::new(&mut iso.cx);
    f(&mut cx)
}

fn dummy_trace(_: ep9_heap::CellPtr, _: &mut ep9_heap::Tracer<'_>) {}

fn dummy_kind(name: &'static str) -> CellKind {
    CellKind {
        name,
        size: CellSize::Fixed(16),
        trace: dummy_trace,
    }
}

// ---- versions and kinds -----------------------------------------------------------------------

#[test]
fn layout_4_and_ic_layout_2_versions() {
    assert_eq!(
        (
            black_box(versions::OBJECT_LAYOUT_VERSION),
            versions::IC_LAYOUT_VERSION
        ),
        (4, 2)
    );
    // The other contract versions do not change in M4-P0 (no bytecode ABI bump, V9-D41).
    assert_eq!(
        (
            versions::FRAME_VERSION,
            versions::BUILTIN_ABI_VERSION,
            versions::DEOPT_META_VERSION,
            ep9_bytecode::ABI_VERSION
        ),
        (2, 1, 0, 5)
    );
}

#[test]
fn the_month_4_kind_ids() {
    let ids = [
        kinds::PROXY,
        kinds::PROXY_CALLABLE,
        kinds::VALIDITY_CELL,
        kinds::WEAK_CELL,
    ];
    assert_eq!(ids.map(KindId::get), [27, 28, 79, 80]);
    assert_eq!(
        [kinds::CONS, kinds::SLICED, kinds::THIN].map(KindId::get),
        [3, 4, 5]
    );
    // PROXY and PROXY_CALLABLE are JS objects (16-63); the cells are internal (64-127).
    assert!((16..=63).contains(&kinds::PROXY.get()));
    assert!((16..=63).contains(&kinds::PROXY_CALLABLE.get()));
    assert!((64..=127).contains(&kinds::VALIDITY_CELL.get()));
    assert!((64..=127).contains(&kinds::WEAK_CELL.get()));
}

/// M4-P0 registers no month-4 kind (the builders do, plan v9 §2.5 task 2): each id is free on
/// a new isolate, and the string ids 3-5 too.
#[test]
fn no_month_4_kind_is_registered_yet() {
    for id in [
        kinds::CONS,
        kinds::SLICED,
        kinds::THIN,
        kinds::PROXY,
        kinds::PROXY_CALLABLE,
        kinds::VALIDITY_CELL,
        kinds::WEAK_CELL,
    ] {
        let iso = isolate(config(Dispatch::DEFAULT, false));
        assert_eq!(
            iso.rt.heap.register_kind(id, dummy_kind("test")),
            Ok(()),
            "kind {} is free",
            id.get()
        );
    }
}

/// IsCallable (§7.2.3) is a kind test that also takes PROXY_CALLABLE (V9-D19), and `is_object`
/// takes both proxy kinds. The cells are made under test kinds with the proxy ids.
#[test]
fn is_callable_takes_proxy_callable() {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let rt = &iso.rt;
    for id in [kinds::PROXY, kinds::PROXY_CALLABLE] {
        rt.heap.register_kind(id, dummy_kind("test proxy")).unwrap();
    }
    let plain = val_bits(rt.alloc(kinds::PROXY, 16));
    let callable = val_bits(rt.alloc(kinds::PROXY_CALLABLE, 16));
    assert!(kinds::is_object(plain) && kinds::is_object(callable));
    assert!(!kinds::is_callable(plain));
    assert!(kinds::is_callable(callable));
    assert!(!kinds::is_function(callable), "a proxy has no FunctionInfo");
    // The cells are unreachable; the next GC frees them.
    iso.rt.heap.collect();
}

fn val_bits(c: ep9_heap::CellPtr) -> u64 {
    let v = c.to_value().to_bits();
    debug_assert_eq!(val(v).to_bits(), v);
    v
}

// ---- ids --------------------------------------------------------------------------------------

#[test]
fn intrinsic_ids_64_to_71_are_declared_not_made() {
    assert_eq!(IntrinsicId::ALL.len(), 72);
    let new = [
        (IntrinsicId::Proxy, "%Proxy%"),
        (IntrinsicId::RegExp, "%RegExp%"),
        (IntrinsicId::RegExpPrototype, "%RegExp.prototype%"),
        (
            IntrinsicId::RegExpStringIteratorPrototype,
            "%RegExpStringIteratorPrototype%",
        ),
        (IntrinsicId::WeakRef, "%WeakRef%"),
        (IntrinsicId::WeakRefPrototype, "%WeakRef.prototype%"),
        (IntrinsicId::FinalizationRegistry, "%FinalizationRegistry%"),
        (
            IntrinsicId::FinalizationRegistryPrototype,
            "%FinalizationRegistry.prototype%",
        ),
    ];
    for (i, (id, name)) in new.iter().enumerate() {
        assert_eq!(*id as usize, 64 + i);
        assert_eq!(id.name(), *name);
        assert_eq!(id.root(), None, "{name} lives in a builtin-state slot");
        assert!(!id.is_core());
    }
    // Nothing makes them in M4-P0: each is undefined in a new realm (and in its REALM
    // record's intrinsics list, which has one entry per id).
    let iso = isolate(config(Dispatch::DEFAULT, false));
    for (id, name) in new {
        assert_eq!(builtins::intrinsic(&iso.rt, id), UNDEFINED, "{name}");
    }
    let realm = iso.rt.root(crate::runtime::Root::Realm);
    let list = object::record_get(realm, object::realmf::INTRINSICS);
    assert_eq!(object::list_len(list), 72);
}

#[test]
fn object_classes_11_to_13() {
    assert_eq!(ObjectClass::ALL.len(), 14);
    let new = [
        (ObjectClass::WeakRef, "WeakRef"),
        (ObjectClass::FinalizationRegistry, "FinalizationRegistry"),
        (ObjectClass::RegExpStringIterator, "RegExp String Iterator"),
    ];
    for (i, (c, name)) in new.into_iter().enumerate() {
        assert_eq!(usize::from(c.id()), 11 + i);
        assert_eq!(c.name(), name);
        assert_eq!(ObjectClass::from_id(c.id()), Some(c));
    }
    assert_eq!(ObjectClass::from_id(14), None);
}

// ---- IC_LAYOUT 2 ------------------------------------------------------------------------------

/// The §2.3 slot table names the slot kinds by ABI byte, and the two new IC_HANDLER kinds
/// follow the four of IC_LAYOUT 1.
#[test]
fn ic_layout_2_constants() {
    use FeedbackSlotKind as F;
    let bytes = [
        (F::Call, 0),
        (F::LoadKeyed, 2),
        (F::StoreKeyed, 4),
        (F::DefineKeyedOwn, 6),
        (F::HasKeyed, 10),
        (F::BinaryOp, 11),
        (F::CompareOp, 12),
        (F::TypeOf, 13),
        (F::InstanceOf, 14),
        (F::Literal, 15),
        (F::CloneObject, 16),
        (F::StoreInArrayLiteral, 17),
        (F::ForIn, 18),
        (F::JumpLoop, 19),
        (F::CreateClosure, 20),
    ];
    for (k, b) in bytes {
        assert_eq!(k as u8, b, "{k:?}");
    }
    assert_eq!(F::ALL.len(), 21);
    let hk = [
        ic::hk::PROTO_LOAD,
        ic::hk::MISSING,
        ic::hk::ADD,
        ic::hk::GLOBAL,
        ic::hk::CONSTRUCT_SITE,
        ic::hk::INSTANCEOF,
    ];
    assert_eq!(hk, [1, 2, 3, 4, 5, 6]);
}

// ---- Runtime fields ---------------------------------------------------------------------------

/// `Cx::math_random` (V9-D16): numbers in [0, 1), multiples of 2^-52, not all equal, and a
/// distinct sequence for each isolate of one process (§21.3.2.27: distinct realms give distinct
/// sequences).
#[test]
fn math_random_is_xorshift128_plus_in_0_1() {
    let a: Vec<f64> = with_cx(false, |cx| (0..1000).map(|_| cx.math_random()).collect());
    let b: Vec<f64> = with_cx(false, |cx| (0..1000).map(|_| cx.math_random()).collect());
    assert_ne!(a[..8], b[..8], "two isolates, two streams");
    for &x in &a {
        assert!((0.0..1.0).contains(&x), "{x}");
        assert_eq!(
            x * 4_503_599_627_370_496.0 % 1.0,
            0.0,
            "{x} is a multiple of 2^-52"
        );
    }
    let distinct: std::collections::BTreeSet<u64> = a.iter().map(|x| x.to_bits()).collect();
    assert!(distinct.len() > 990, "{} distinct of 1000", distinct.len());
    let mean = a.iter().sum::<f64>() / a.len() as f64;
    assert!((0.4..0.6).contains(&mean), "mean {mean}");
}

#[test]
fn the_rng_seed_is_never_zero() {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    let s = crate::runtime::rng_seed(iso.rt.id);
    assert_ne!(s, [0, 0]);
    assert_eq!(iso.rt.rng.get(), s, "the seed of the isolate's id");
    assert_ne!(
        s,
        crate::runtime::rng_seed(iso.rt.id + 1),
        "another id, another seed"
    );
    for id in 0..64 {
        assert_ne!(crate::runtime::rng_seed(id), [0, 0], "id {id}");
    }
}

/// The first `Math.random` numbers (as bits) of two new isolates of this process.
fn random_bits_of_two_new_isolates() -> [Vec<u64>; 2] {
    let one = || {
        with_cx(false, |cx| {
            (0..4).map(|_| cx.math_random().to_bits()).collect()
        })
    };
    [one(), one()]
}

/// Child of `one_hash_seed_gives_one_math_random_run`: prints the `Math.random` bits of two
/// new isolates when `EP9_RNG_CHILD=rng`.
#[test]
fn math_random_seed_child() {
    if std::env::var("EP9_RNG_CHILD").as_deref() != Ok("rng") {
        return;
    }
    println!("RNGBITS {:?}", random_bits_of_two_new_isolates());
}

#[cfg(not(miri))]
fn child_random_bits(seed: &str) -> String {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "tests::layout4::math_random_seed_child",
            "--exact",
            "--nocapture",
            "--test-threads",
            "1",
        ])
        .env("EP9_RNG_CHILD", "rng")
        .env("EP9_HASH_SEED", seed)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    let i = text
        .find("RNGBITS ")
        .expect("the child printed its numbers");
    text[i..].lines().next().unwrap().to_owned()
}

/// V9-D16: two runs with one `EP9_HASH_SEED` give the same `Math.random` numbers isolate by
/// isolate (the n-th isolate of the run has one seed); the two isolates of one run differ;
/// another seed gives other numbers.
#[test]
#[cfg(not(miri))]
fn one_hash_seed_gives_one_math_random_run() {
    let one = child_random_bits("1");
    assert_eq!(one, child_random_bits("1"), "EP9_HASH_SEED=1 twice");
    assert_ne!(one, child_random_bits("2"), "another seed, other numbers");
    let [a, b] = random_bits_of_two_new_isolates();
    assert_ne!(a, b, "two isolates of one process");
}

/// `Runtime::kept_objects` is a strong root slot: a list stored there (by a function that has
/// returned, so no Rust local holds it) survives collections under GC stress and the heap
/// verifier; undefined at the start.
#[test]
fn kept_objects_is_a_root_slot() {
    #[inline(never)]
    fn keep_a_list(rt: &crate::runtime::Runtime) {
        let list = object::new_list(rt, 3, UNDEFINED);
        rt.kept_objects.set(list);
        let o = object::new_plain_object(rt);
        object::list_set(rt, list, 1, o);
    }
    let iso = isolate(config(Dispatch::DEFAULT, true));
    let rt = &iso.rt;
    assert_eq!(rt.kept_objects.get(), UNDEFINED);
    keep_a_list(rt);
    for _ in 0..3 {
        rt.heap.collect();
    }
    iso.verify_heap().unwrap();
    let list = rt.kept_objects.get();
    assert_eq!(object::list_len(list), 3);
    assert!(kinds::is_object(object::list_get(list, 1)));
    rt.kept_objects.set(UNDEFINED);
    rt.heap.collect();
    iso.verify_heap().unwrap();
}
