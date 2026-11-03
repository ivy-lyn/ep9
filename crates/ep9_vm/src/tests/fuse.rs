//! Tests of the fuse registry v0 (`fuse.rs`; M5-P1 FR0, H31, C-01): a pop invalidates every
//! dependent and is one-way, a dead dependent is dropped, a proxy chain is refused
//! (MC-w16-15), a descriptor must name its subject, the constructor-chain kind (MC-w15-9),
//! changes that must not pop, two realms (one word per subject, dependents per isolate:
//! H31-Q2, M5-P1-ORCH-13; and a read for a subject no arm names), the static
//! list order, the weak and strong handles across collections, and the runtime sweep at the
//! cleanup point after a GC. Each test runs with and without GC stress (a collection
//! before every allocation, then the heap verifier).

use ep9_facts::{FunctionRef, ProtoRef};

use super::ic::{int, key, object_prototype, object_with};
use super::{config, isolate, on_big_stack};
use crate::builtins::{self, BuiltinId, IntrinsicId};
use crate::frame::NULL;
use crate::fuse::{
    CodeHandle, Descriptor, FuseRef, NO_ELEMENTS_PROTOTYPES, Refusal, STATIC, descriptor,
};
use crate::object::{self, attr};
use crate::runtime::{Root, Runtime};
use crate::{Dispatch, Isolate, fuse, layout, natives, property, props, proxy, realm, shape};

/// Runs `test` on a fresh isolate, with and without GC stress, on a big stack; collects and
/// verifies the heap after it.
fn each(test: fn(&mut Isolate)) {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            test(&mut iso);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// Adds the data property `name` {w, e, c} to `o`: a structural change.
fn add(rt: &Runtime, o: u64, name: &str) {
    props::own_define(rt, o, key(rt, name), int(1), attr::DEFAULT);
}

fn proto_chain(id: IntrinsicId) -> Descriptor {
    Descriptor::ProtoChain(ProtoRef::Intrinsic(id as u32))
}

/// A table builtin of the VM table whose function object exists, with its id.
fn some_builtin(rt: &Runtime) -> (BuiltinId, u64) {
    let m = &natives::VM_TABLE.modules[0];
    (0..m.defs.len())
        .find_map(|d| {
            let id = m.id(d as u16);
            builtins::existing_function(rt, id).map(|f| (id, f))
        })
        .expect("a VM builtin function")
}

#[test]
fn a_pop_invalidates_every_dependent() {
    each(|iso| {
        let rt = &iso.rt;
        let fuses = &rt.fuses;
        let op = object_prototype(rt);
        let ap = builtins::intrinsic(rt, IntrinsicId::ArrayPrototype);
        let d = proto_chain(IntrinsicId::ArrayPrototype);
        let f = FuseRef::Dynamic(d);
        let word = fuses.arm(rt, d, ap).unwrap();
        assert!(word.intact() && fuses.intact(rt, f, ap));
        let code: Vec<CodeHandle> = (0..3).map(|_| CodeHandle::new()).collect();
        for c in &code {
            fuses.depend(rt, f, ap, c).unwrap();
        }
        // The same handle twice is one entry.
        fuses.depend(rt, f, ap, &code[0]).unwrap();
        assert_eq!(fuses.dependent_entries(f), 3);
        // A change above the subject (Object.prototype is on the chain) pops it.
        add(rt, op, "fuseA");
        assert!(code.iter().all(CodeHandle::is_invalidated));
        assert!(!word.intact() && !fuses.intact(rt, f, ap));
        // One-way: no new dependent, no new arm, the word stays popped.
        let late = CodeHandle::new();
        assert_eq!(fuses.depend(rt, f, ap, &late), Err(Refusal::Popped));
        assert_eq!(fuses.arm(rt, d, ap).err(), Some(Refusal::Popped));
        assert_eq!(fuses.guard(rt, f, ap).err(), Some(Refusal::Popped));
        assert!(!late.is_invalidated());
        add(rt, ap, "fuseB");
        assert!(!word.intact());
        assert!(fuses.is_empty());

        // A static fuse: wired, three dependents, an explicit pop; the others stay.
        let (s, other) = (FuseRef::Static(7), FuseRef::Static(8));
        fuses.wire(rt, 7).unwrap();
        fuses.wire(rt, 8).unwrap();
        let sc: Vec<CodeHandle> = (0..3).map(|_| CodeHandle::new()).collect();
        // A static fuse ignores the subject (NULL here).
        for c in &sc {
            fuses.depend(rt, s, NULL, c).unwrap();
        }
        let oc = CodeHandle::new();
        fuses.depend(rt, other, NULL, &oc).unwrap();
        let sw = fuses.guard(rt, s, NULL).unwrap();
        fuses.pop(rt, s);
        assert!(sc.iter().all(CodeHandle::is_invalidated));
        assert!(!sw.intact() && !fuses.intact(rt, s, NULL));
        assert!(fuses.intact(rt, other, ap) && !oc.is_invalidated());
        assert_eq!(
            fuses.depend(rt, s, NULL, &CodeHandle::new()),
            Err(Refusal::Popped)
        );
        // A second pop changes nothing.
        fuses.pop(rt, s);
        assert!(!sw.intact());
    });
}

#[test]
fn a_dead_dependent_is_dropped() {
    each(|iso| {
        let rt = &iso.rt;
        let fuses = &rt.fuses;
        let ap = builtins::intrinsic(rt, IntrinsicId::ArrayPrototype);
        let d = proto_chain(IntrinsicId::ArrayPrototype);
        let f = FuseRef::Dynamic(d);
        let _word = fuses.arm(rt, d, ap).unwrap();
        let live = CodeHandle::new();
        fuses.depend(rt, f, ap, &live).unwrap();
        // 100 code objects that die at once: a full list drops its dead entries before it
        // grows, so it never holds them all.
        for _ in 0..100 {
            let c = CodeHandle::new();
            fuses.depend(rt, f, ap, &c).unwrap();
            drop(c);
        }
        assert!(
            fuses.dependent_entries(f) <= 8,
            "a full list drops the dead ones"
        );
        fuses.sweep(rt);
        assert_eq!(fuses.dependent_entries(f), 1, "only the live one stays");
        // A clone keeps the code alive; the last drop kills it.
        let clone = live.clone();
        drop(live);
        fuses.sweep(rt);
        assert_eq!(fuses.dependent_entries(f), 1);
        drop(clone);
        fuses.sweep(rt);
        assert_eq!(fuses.dependent_entries(f), 0);
        // A pop with dead dependents only: nothing to mark, no panic.
        let dead = CodeHandle::new();
        fuses.depend(rt, f, ap, &dead).unwrap();
        drop(dead);
        add(rt, ap, "fuseC");
        assert!(!fuses.intact(rt, f, ap));
    });
}

#[test]
fn an_unused_fuse_is_swept_and_a_held_word_keeps_it() {
    each(|iso| {
        let rt = &iso.rt;
        let fuses = &rt.fuses;
        let ap = builtins::intrinsic(rt, IntrinsicId::ArrayPrototype);
        let d = proto_chain(IntrinsicId::ArrayPrototype);
        let f = FuseRef::Dynamic(d);
        let word = fuses.arm(rt, d, ap).unwrap();
        fuses.sweep(rt);
        assert!(fuses.intact(rt, f, ap), "a GuardFuse reader holds the word");
        drop(word);
        fuses.sweep(rt);
        assert_eq!(fuses.guard(rt, f, ap).err(), Some(Refusal::NotArmed));
        assert!(fuses.is_empty());
        // Swept, not popped: a later arm makes it again.
        let word = fuses.arm(rt, d, ap).unwrap();
        assert!(word.intact());
    });
}

#[test]
fn a_proxy_chain_is_refused() {
    each(|iso| {
        let r = iso.with_cx(|cx| {
            let target = cx.new_object().bits();
            let handler = cx.new_object().bits();
            let inner = cx.inner();
            let p = proxy::proxy_create(inner, target, handler).expect("proxy_create");
            let rt: &Runtime = inner.rt();
            let fuses = &rt.fuses;
            // Array.prototype -> a proxy -> (its trap): the chain can change with no
            // structural change, so no validity cell guards it.
            let ap = builtins::intrinsic(rt, IntrinsicId::ArrayPrototype);
            let d = proto_chain(IntrinsicId::ArrayPrototype);
            assert!(object::set_proto(rt, ap, p));
            assert_eq!(fuses.arm(rt, d, ap).err(), Some(Refusal::ProxyOnChain));
            assert_eq!(
                fuses.depend(rt, FuseRef::Dynamic(d), ap, &CodeHandle::new()),
                Err(Refusal::NotArmed)
            );
            // Object.prototype's chain has no proxy: armed.
            let od = proto_chain(IntrinsicId::ObjectPrototype);
            let op = builtins::intrinsic(rt, IntrinsicId::ObjectPrototype);
            assert!(fuses.arm(rt, od, op).is_ok());
            // A callable proxy is no FUNCTION: the constructor kind names no proxy.
            let (id, _) = some_builtin(rt);
            let d2 = Descriptor::ConstructorChain(FunctionRef::Builtin(id.bits()));
            assert_eq!(fuses.arm(rt, d2, p).err(), Some(Refusal::NotNamed));
            Ok(())
        });
        assert_eq!(r, Ok(()));
    });
}

#[test]
fn a_descriptor_must_name_its_subject() {
    each(|iso| {
        let rt = &iso.rt;
        let fuses = &rt.fuses;
        let op = object_prototype(rt);
        let ap = builtins::intrinsic(rt, IntrinsicId::ArrayPrototype);
        let ad = proto_chain(IntrinsicId::ArrayPrototype);
        assert_eq!(fuses.arm(rt, ad, op).err(), Some(Refusal::NotNamed));
        assert_eq!(fuses.arm(rt, ad, NULL).err(), Some(Refusal::NotNamed));
        // An index past the intrinsic list.
        let past = Descriptor::ProtoChain(ProtoRef::Intrinsic(u32::MAX));
        assert_eq!(fuses.arm(rt, past, ap).err(), Some(Refusal::NotNamed));
        // `Unknown` names nothing.
        let unknown = Descriptor::ProtoChain(ProtoRef::Unknown);
        let plain = object_with(rt, op, &[("a", int(0))]);
        assert_eq!(fuses.arm(rt, unknown, plain).err(), Some(Refusal::NotNamed));
        // A FunctionPrototype name needs the class proof: a plain object whose
        // `constructor` is a function is not one.
        let (id, bf) = some_builtin(rt);
        let fake = object_with(rt, op, &[]);
        props::own_define(rt, fake, key(rt, "constructor"), bf, attr::DEFAULT);
        let fp =
            Descriptor::ProtoChain(ProtoRef::FunctionPrototype(FunctionRef::Builtin(id.bits())));
        assert_eq!(fuses.arm(rt, fp, fake).err(), Some(Refusal::NotNamed));
        // A unit function name on a builtin, another builtin's id.
        let js = Descriptor::ConstructorChain(FunctionRef::Js {
            unit: [0; 32],
            function: 1,
        });
        assert_eq!(fuses.arm(rt, js, bf).err(), Some(Refusal::NotNamed));
        let other = BuiltinId::new(id.block(), id.index() ^ 1);
        let wrong = Descriptor::ConstructorChain(FunctionRef::Builtin(other.bits()));
        if builtins::existing_function(rt, other) != Some(bf) {
            assert_eq!(fuses.arm(rt, wrong, bf).err(), Some(Refusal::NotNamed));
        }
        // The null chain: armed, and nothing pops it.
        let nd = Descriptor::ProtoChain(ProtoRef::Null);
        let nw = fuses.arm(rt, nd, NULL).unwrap();
        add(rt, op, "fuseD");
        assert!(nw.intact() && fuses.intact(rt, FuseRef::Dynamic(nd), NULL));
        assert_eq!(fuses.subject_count(nd), 1);
    });
}

#[test]
fn changes_below_the_subject_off_the_chain_and_value_stores_do_not_pop() {
    each(|iso| {
        let rt = &iso.rt;
        let fuses = &rt.fuses;
        let op = builtins::intrinsic(rt, IntrinsicId::ObjectPrototype);
        let ap = builtins::intrinsic(rt, IntrinsicId::ArrayPrototype);
        let fp = builtins::intrinsic(rt, IntrinsicId::FunctionPrototype);
        let (od, ad) = (
            proto_chain(IntrinsicId::ObjectPrototype),
            proto_chain(IntrinsicId::ArrayPrototype),
        );
        let ow = fuses.arm(rt, od, op).unwrap();
        let aw = fuses.arm(rt, ad, ap).unwrap();
        // Off the chain of either: Function.prototype and a new prototype under
        // Object.prototype.
        add(rt, fp, "fuseE");
        let q = object_with(rt, op, &[("a", int(0))]);
        let _child = object::new_object(rt, q);
        add(rt, q, "b");
        assert!(ow.intact() && aw.intact(), "off both chains: no pop");
        // A value store to a property on both chains is no structural change.
        assert!(props::own_replace(rt, op, key(rt, "toString"), int(3)));
        assert!(ow.intact() && aw.intact(), "a value store does not pop");
        // A change of Array.prototype is below Object.prototype: only its own fuse pops.
        add(rt, ap, "fuseF");
        assert!(!aw.intact(), "a change of the subject itself pops");
        assert_eq!(fuses.arm(rt, ad, ap).err(), Some(Refusal::Popped));
        assert!(ow.intact() && fuses.intact(rt, FuseRef::Dynamic(od), op));
        add(rt, op, "fuseG");
        assert!(!ow.intact());
    });
}

/// MODES-CHECK-w18 MC-w18-9 (M5-P1 fixer): the hook gets only the changed prototype, so a
/// chain fuse must pop when any prototype of its chain changes, the middle one too. A
/// 3-level chain: RangeError.prototype -> Error.prototype -> Object.prototype.
#[test]
fn a_change_in_the_middle_of_a_three_level_chain_pops() {
    each(|iso| {
        let rt = &iso.rt;
        let fuses = &rt.fuses;
        let rp = builtins::intrinsic(rt, IntrinsicId::RangeErrorPrototype);
        let ep = builtins::intrinsic(rt, IntrinsicId::ErrorPrototype);
        let op = builtins::intrinsic(rt, IntrinsicId::ObjectPrototype);
        assert_eq!(object::proto_of(rp), ep);
        assert_eq!(object::proto_of(ep), op);
        let d = proto_chain(IntrinsicId::RangeErrorPrototype);
        let w = fuses.arm(rt, d, rp).unwrap();
        let code = CodeHandle::new();
        fuses.depend(rt, FuseRef::Dynamic(d), rp, &code).unwrap();
        add(rt, ep, "fuseM");
        assert!(
            !w.intact() && code.is_invalidated(),
            "the middle prototype changed"
        );
    });
}

#[test]
fn the_constructor_chain_pops_on_the_function_and_above() {
    for through_function in [true, false] {
        for stress in [false, true] {
            on_big_stack(move || {
                let mut iso = isolate(config(Dispatch::DEFAULT, stress));
                {
                    let rt = &iso.rt;
                    let fuses = &rt.fuses;
                    let (id, f) = some_builtin(rt);
                    let d = Descriptor::ConstructorChain(FunctionRef::Builtin(id.bits()));
                    assert_eq!(d.kind(), descriptor::CONSTRUCTOR_CHAIN);
                    let w = fuses.arm(rt, d, f).unwrap();
                    assert!(
                        shape::is_prototype_object(f),
                        "the arm puts F in prototype mode"
                    );
                    let c = CodeHandle::new();
                    fuses.depend(rt, FuseRef::Dynamic(d), f, &c).unwrap();
                    // A change of an unrelated prototype does not pop it.
                    add(
                        rt,
                        builtins::intrinsic(rt, IntrinsicId::ArrayPrototype),
                        "fuseH",
                    );
                    assert!(w.intact());
                    if through_function {
                        add(rt, f, "fuseI");
                    } else {
                        // The function's [[Prototype]] (an intrinsic of the realm).
                        let fp = object::proto_of(f);
                        assert!(object::is_object(fp));
                        add(rt, fp, "fuseJ");
                    }
                    assert!(!w.intact() && c.is_invalidated());
                }
                iso.collect_garbage();
                iso.verify_heap().unwrap();
            });
        }
    }
}

/// H31-Q2 (M5-P1-ORCH-13, built in M5-P2 CONSTREP): one fuse armed in two realms keeps one
/// word per subject and one dependents set per isolate. A pop in realm A (either one)
/// invalidates the dependents armed through both realms and refuses new ones (the "popped in
/// any realm" summary); realm A's word reads popped and its subject never arms again; realm
/// B's word and `intact` stay true, and B's subject still arms (a GuardFuse reader in B keeps
/// its fast path).
#[test]
fn a_pop_in_realm_a_invalidates_every_dependent_and_realm_b_stays_intact() {
    for second_pops in [true, false] {
        for stress in [false, true] {
            on_big_stack(move || {
                let mut iso = isolate(config(Dispatch::DEFAULT, stress));
                let r = iso.with_cx(|cx| {
                    let g = cx.create_realm()?;
                    let d = proto_chain(IntrinsicId::ObjectPrototype);
                    let f = FuseRef::Dynamic(d);
                    let (c1, c2) = (CodeHandle::new(), CodeHandle::new());
                    let (w1, op1) = {
                        let rt: &Runtime = cx.inner().rt();
                        let op = builtins::intrinsic(rt, IntrinsicId::ObjectPrototype);
                        let w = rt.fuses.arm(rt, d, op).unwrap();
                        rt.fuses.depend(rt, f, op, &c1).unwrap();
                        (w, op)
                    };
                    let (w2, op2) = cx.with_realm(g, |cx| {
                        let rt: &Runtime = cx.inner().rt();
                        let op = builtins::intrinsic(rt, IntrinsicId::ObjectPrototype);
                        assert_ne!(op, op1);
                        // One fuse, a second subject with its own word.
                        let w2 = rt.fuses.arm(rt, d, op).unwrap();
                        assert!(!std::rc::Rc::ptr_eq(&w1, &w2));
                        assert_eq!(rt.fuses.subject_count(d), 2);
                        rt.fuses.depend(rt, f, op, &c2).unwrap();
                        rt.heap.collect();
                        Ok((w2, op))
                    })?;
                    let rt: &Runtime = cx.inner().rt();
                    assert!(w1.intact() && w2.intact());
                    let (popped, kept, kw, pw) = if second_pops {
                        (op2, op1, &w1, &w2)
                    } else {
                        (op1, op2, &w2, &w1)
                    };
                    add(rt, popped, "fuseK");
                    // Every dependent, armed through either realm, is invalidated.
                    assert!(c1.is_invalidated() && c2.is_invalidated());
                    assert!(!pw.intact() && !rt.fuses.intact(rt, f, popped));
                    assert_eq!(rt.fuses.guard(rt, f, popped).err(), Some(Refusal::Popped));
                    // The other realm's word stays intact, and its subject still arms (an
                    // arm names the subject in the current realm: realm 1 here).
                    assert!(kw.intact() && rt.fuses.intact(rt, f, kept));
                    if second_pops {
                        let again = rt.fuses.arm(rt, d, kept).unwrap();
                        assert!(std::rc::Rc::ptr_eq(&again, kw));
                    } else {
                        assert_eq!(rt.fuses.arm(rt, d, popped).err(), Some(Refusal::Popped));
                    }
                    assert_eq!(rt.fuses.subject_count(d), 1);
                    // No new dependent through either realm (popped in any realm).
                    for s in [kept, popped] {
                        assert_eq!(
                            rt.fuses.depend(rt, f, s, &CodeHandle::new()),
                            Err(Refusal::Popped)
                        );
                    }
                    // The kept subject still pops on its own change.
                    add(rt, kept, "fuseK2");
                    assert!(!kw.intact());
                    Ok(())
                });
                assert_eq!(r, Ok(()));
                iso.collect_garbage();
                iso.verify_heap().unwrap();
            });
        }
    }
}

/// A fuse armed in realm 1 answers for realm 1's subject only (M5-P1 review, Codex r1 fr0):
/// realm 2's Array.prototype, which no arm names, reads not armed for `guard`, `intact` and
/// `depend`, while realm 2's chain changes (a new property, then a proxy put on it) and pop
/// nothing; realm 1's subject still reads intact. Arming realm 2's subject with a proxy on
/// its chain is refused.
#[test]
fn a_subject_of_another_realm_is_not_armed_by_the_first_arm() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let r = iso.with_cx(|cx| {
                let g = cx.create_realm()?;
                let d = proto_chain(IntrinsicId::ArrayPrototype);
                let f = FuseRef::Dynamic(d);
                let c = CodeHandle::new();
                let (w, ap1) = {
                    let rt: &Runtime = cx.inner().rt();
                    let ap = builtins::intrinsic(rt, IntrinsicId::ArrayPrototype);
                    let w = rt.fuses.arm(rt, d, ap).unwrap();
                    rt.fuses.depend(rt, f, ap, &c).unwrap();
                    (w, ap)
                };
                cx.with_realm(g, |cx| {
                    let target = cx.new_object().bits();
                    let handler = cx.new_object().bits();
                    let inner = cx.inner();
                    let p = proxy::proxy_create(inner, target, handler).expect("proxy_create");
                    let rt: &Runtime = inner.rt();
                    let fuses = &rt.fuses;
                    let ap2 = builtins::intrinsic(rt, IntrinsicId::ArrayPrototype);
                    assert_ne!(ap2, ap1);
                    let not_armed = |why: &str| {
                        assert_eq!(
                            fuses.guard(rt, f, ap2).err(),
                            Some(Refusal::NotArmed),
                            "{why}"
                        );
                        assert!(!fuses.intact(rt, f, ap2), "{why}");
                        assert_eq!(
                            fuses.depend(rt, f, ap2, &CodeHandle::new()),
                            Err(Refusal::NotArmed),
                            "{why}"
                        );
                    };
                    not_armed("never armed");
                    // Realm 2's chain changes: nothing watches it, so nothing pops.
                    add(rt, ap2, "fuseN");
                    not_armed("a new property");
                    assert!(object::set_proto(rt, ap2, p));
                    not_armed("a proxy on the chain");
                    assert_eq!(fuses.arm(rt, d, ap2).err(), Some(Refusal::ProxyOnChain));
                    not_armed("the arm was refused");
                    assert!(w.intact() && !c.is_invalidated());
                    assert!(fuses.intact(rt, f, ap1));
                    assert_eq!(fuses.subject_count(d), 1);
                    Ok(())
                })?;
                // Realm 1's chain still pops the fuse.
                let rt: &Runtime = cx.inner().rt();
                add(rt, ap1, "fuseO");
                assert!(!w.intact() && c.is_invalidated());
                Ok(())
            });
            assert_eq!(r, Ok(()));
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// The runtime sweep (M5-P1 fixer round 3, Codex r1 fr0 medium): once every user of a dynamic
/// fuse is gone (its word and its code handles), the first FinalizationRegistry cleanup point
/// after a GC (`Cx::cleanup_finalization_registries`, which every microtask checkpoint runs)
/// drops the fuse and its handles, with no `Registry::sweep` call here. A drop alone and a
/// GC alone keep the entry; a fuse whose word is still held stays through both.
#[test]
fn an_unused_fuse_goes_at_the_cleanup_point_after_a_gc() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let cleanup = |iso: &mut Isolate| {
                iso.with_cx(|cx| cx.cleanup_finalization_registries())
                    .expect("no registry, no callback");
            };
            let ap = builtins::intrinsic(&iso.rt, IntrinsicId::ArrayPrototype);
            let op = object_prototype(&iso.rt);
            let unused = proto_chain(IntrinsicId::ArrayPrototype);
            let held = proto_chain(IntrinsicId::ObjectPrototype);
            let held_word = {
                let rt = &iso.rt;
                let w = rt.fuses.arm(rt, unused, ap).unwrap();
                let c = CodeHandle::new();
                rt.fuses
                    .depend(rt, FuseRef::Dynamic(unused), ap, &c)
                    .unwrap();
                drop((w, c));
                rt.fuses.arm(rt, held, op).unwrap()
            };
            // Every user of `unused` is gone; nothing has swept yet.
            assert_eq!(iso.rt.fuses.subject_count(unused), 1);
            assert_eq!(iso.rt.fuses.dependent_entries(FuseRef::Dynamic(unused)), 1);
            iso.collect_garbage();
            assert_eq!(
                iso.rt.fuses.subject_count(unused),
                1,
                "a GC alone sweeps nothing"
            );
            cleanup(&mut iso);
            let rt = &iso.rt;
            assert_eq!(rt.fuses.subject_count(unused), 0, "stress={stress}");
            assert_eq!(rt.fuses.dependent_entries(FuseRef::Dynamic(unused)), 0);
            assert!(!rt.fuses.intact(rt, FuseRef::Dynamic(unused), ap));
            // The held fuse stays armed and intact.
            assert_eq!(rt.fuses.subject_count(held), 1);
            assert!(held_word.intact() && rt.fuses.intact(rt, FuseRef::Dynamic(held), op));
            drop(held_word);
            iso.collect_garbage();
            cleanup(&mut iso);
            assert!(iso.rt.fuses.is_empty(), "stress={stress}");
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

#[test]
fn an_armed_fuse_survives_collections() {
    each(|iso| {
        let ap = builtins::intrinsic(&iso.rt, IntrinsicId::ArrayPrototype);
        let d = proto_chain(IntrinsicId::ArrayPrototype);
        let w = iso.rt.fuses.arm(&iso.rt, d, ap).unwrap();
        let c = CodeHandle::new();
        iso.rt
            .fuses
            .depend(&iso.rt, FuseRef::Dynamic(d), ap, &c)
            .unwrap();
        for _ in 0..3 {
            iso.collect_garbage();
        }
        iso.verify_heap().unwrap();
        let rt = &iso.rt;
        // The cell and the subject live through the handles (the subject is an intrinsic).
        rt.fuses.sweep(rt);
        assert_eq!(rt.fuses.subject_count(d), 1);
        assert!(w.intact());
        add(
            rt,
            builtins::intrinsic(rt, IntrinsicId::ObjectPrototype),
            "fuseL",
        );
        assert!(!w.intact() && c.is_invalidated());
    });
}

#[test]
fn static_fuses_keep_their_order_and_need_a_pop_site() {
    // The H31 Decision list order (append-only; an index never changes).
    assert_eq!(
        STATIC,
        [
            "Array species",
            "Array iterator",
            "String iterator",
            "Map iterator",
            "Set iterator",
            "Promise.then lookup",
            "no-elements prototypes",
            "RegExp exec",
            "TypedArray species",
            "InspectorSession",
            "DebuggerBindingWrite",
        ]
    );
    each(|iso| {
        let rt = &iso.rt;
        let fuses = &rt.fuses;
        let n = STATIC.len() as u32;
        // One pop site is wired (M6-P2 FUSE-PI): index 6 "no-elements prototypes" reads
        // intact in a new isolate. Nothing may use another static fuse: each reads not intact.
        assert_eq!(
            STATIC[NO_ELEMENTS_PROTOTYPES as usize],
            "no-elements prototypes"
        );
        for i in 0..n {
            let f = FuseRef::Static(i);
            if i == NO_ELEMENTS_PROTOTYPES {
                assert!(fuses.intact(rt, f, NULL));
                continue;
            }
            assert!(!fuses.intact(rt, f, NULL));
            assert_eq!(fuses.guard(rt, f, NULL).err(), Some(Refusal::NotWired));
            assert_eq!(
                fuses.depend(rt, f, NULL, &CodeHandle::new()),
                Err(Refusal::NotWired)
            );
        }
        let past = FuseRef::Static(u32::MAX);
        assert_eq!(fuses.wire(rt, u32::MAX), Err(Refusal::UnknownStatic));
        assert_eq!(
            fuses.guard(rt, past, NULL).err(),
            Some(Refusal::UnknownStatic)
        );
        assert_eq!(
            fuses.depend(rt, past, NULL, &CodeHandle::new()),
            Err(Refusal::UnknownStatic)
        );
        fuses.pop(rt, past);
        // Wired: intact until popped, then never again.
        fuses.wire(rt, 0).unwrap();
        let w = fuses.guard(rt, FuseRef::Static(0), NULL).unwrap();
        assert!(w.intact());
        fuses.pop(rt, FuseRef::Static(0));
        fuses.wire(rt, 0).unwrap();
        assert!(!w.intact() && !fuses.intact(rt, FuseRef::Static(0), NULL));
    });
}

#[test]
fn descriptor_kinds_are_a_known_list() {
    assert_eq!(
        Descriptor::ProtoChain(ProtoRef::Null).kind(),
        descriptor::PROTO_CHAIN
    );
    assert!(descriptor::known(descriptor::PROTO_CHAIN));
    assert!(descriptor::known(descriptor::CONSTRUCTOR_CHAIN));
    // An unknown kind is refused (never probe the next free one: H31 Freeze table).
    for k in [0, 0x7f, 0xff] {
        assert!(!descriptor::known(k));
    }
}

// ---- the static fuse "no-elements prototypes" and the realm fuse block (M6-P2 FUSE-PI) -----
//
// H31-Q5, V11-D21, MODES-CHECK-w25 MC-w25-11. The fuse replaces the sticky `proto_index`
// flag; `shape::note_proto_index_key` is its one writer and pops it in every realm (a
// prototype chain can cross realms, plan v11 §13 R19). Each test runs with GC stress off and
// on (a collection before every allocation), then the heap verifier.

/// The index of "no-elements prototypes" in a realm block.
const NE: usize = NO_ELEMENTS_PROTOTYPES as usize;

/// The u32 words of the realm block `b` (a FUSE cell).
fn block_words(b: u64) -> Vec<u32> {
    assert!(crate::kinds::is_cell_bits(b), "a realm block is a cell");
    assert_eq!(crate::kinds::cell_kind(b), crate::kinds::FUSE.get());
    let c = object::cell(b);
    (0..STATIC.len())
        .map(|i| {
            let off = (layout::FUSE_ENTRIES + layout::FUSE_ENTRY_SIZE * i as u32) as usize;
            // SAFETY: `b` is a live FUSE cell (a realm holds it) and entry `i` is inside it.
            unsafe { c.field::<u32>(off).read() }
        })
        .collect()
}

/// The realm block of the REALM record `r`, from its intrinsics list.
fn block_of(rt: &Runtime, r: u64) -> u64 {
    realm::intrinsic_in(rt, r, IntrinsicId::RealmFuses)
}

/// The words of a new isolate's blocks: only "no-elements prototypes" is wired and intact.
fn fresh_words() -> Vec<u32> {
    (0..STATIC.len()).map(|i| u32::from(i == NE)).collect()
}

/// The words after a pop of "no-elements prototypes".
fn popped_words() -> Vec<u32> {
    vec![0; STATIC.len()]
}

/// Each realm gets its own block, made with the realm (`realm::init_core`): a FUSE cell (kind
/// 83, 56 bytes) in `Root::RealmFuses` and in the realm's intrinsics list, whose entries are
/// the isolate's static words (1 = wired and intact). A realm switch carries the root. A
/// `wire` and a static pop write the entry in every realm's block.
#[test]
fn each_realm_has_a_block_with_the_static_words() {
    each(|iso| {
        let r = iso.with_cx(|cx| {
            // R6-2: no embedder sees the block as an intrinsic object.
            assert!(!cx.has_intrinsic(IntrinsicId::RealmFuses));
            let first = cx.inner().realm;
            let b1 = {
                let rt: &Runtime = cx.inner().rt();
                let b1 = rt.root(Root::RealmFuses);
                assert_eq!(block_of(rt, first), b1, "the record mirrors the root");
                assert_eq!(builtins::intrinsic(rt, IntrinsicId::RealmFuses), b1);
                assert!(!crate::kinds::is_object(b1), "not a JS value");
                // SAFETY: `b1` is a live cell of this heap.
                let size = unsafe { rt.heap.cell_size(object::cell(b1)) };
                assert_eq!(size, layout::FUSE_SIZE as usize);
                assert_eq!(block_words(b1), fresh_words());
                assert!(fuse::realm_intact(rt, NO_ELEMENTS_PROTOTYPES));
                assert!(!fuse::realm_intact(rt, 0), "an unwired fuse reads 0");
                assert!(!shape::proto_index_keys(rt));
                assert_eq!(rt.fuses.block_count(), 1);
                b1
            };
            let g = cx.create_realm()?;
            let rt: &Runtime = cx.inner().rt();
            let second = realm::of_global(rt, g.bits()).expect("a realm's global");
            let b2 = block_of(rt, second);
            assert_ne!(b1, b2, "a realm's own block");
            assert_eq!(block_words(b2), fresh_words());
            assert_eq!(rt.fuses.block_count(), 2);
            assert_eq!(
                rt.root(Root::RealmFuses),
                b1,
                "create_realm leaves realm 1 current"
            );
            cx.with_realm(g, |cx| {
                let rt: &Runtime = cx.inner().rt();
                assert_eq!(rt.root(Root::RealmFuses), b2, "the switch loads the root");
                assert!(!shape::proto_index_keys(rt));
                rt.heap.collect();
                Ok(())
            })?;
            let rt: &Runtime = cx.inner().rt();
            assert_eq!(rt.root(Root::RealmFuses), b1, "and puts realm 1's back");
            // `wire` and a static pop write every realm's entry.
            rt.fuses.wire(rt, 0).unwrap();
            for b in [b1, b2] {
                assert_eq!(block_words(b)[0], layout::FUSE_INTACT);
            }
            assert!(fuse::realm_intact(rt, 0));
            rt.fuses.pop(rt, FuseRef::Static(0));
            for b in [b1, b2] {
                assert_eq!(block_words(b), fresh_words());
            }
            assert!(!fuse::realm_intact(rt, 0) && !rt.fuses.intact(rt, FuseRef::Static(0), NULL));
            rt.fuses.wire(rt, 0).unwrap();
            assert_eq!(block_words(b1)[0], 0, "a popped fuse stays popped");
            Ok(())
        });
        assert_eq!(r, Ok(()));
    });
}

/// A way an index key reaches a prototype: it makes a prototype-mode object `p` in the current
/// realm and gives `p` an array-index own property (or makes an object that has one a
/// prototype).
type Way = fn(&mut crate::interp::Cx);

/// Every way an index key reaches a prototype (MC-w25-11; `shape::note_proto_index_key` has
/// these 5 callers), with the caller each one reaches.
const WAYS: [(&str, Way); 6] = [
    (
        "object.rs array_set_sparse: a DICTIONARY entry",
        way_sparse_element,
    ),
    ("object.rs store_element_slow: an element", way_element),
    (
        "property.rs dict_define: an entry with attributes",
        way_dict_define,
    ),
    ("props.rs own_add: a stored index key", way_stored_key),
    (
        "shape.rs proto_info_for: an array with an element",
        way_becomes_prototype,
    ),
    (
        "shape.rs proto_info_for: a routed object (a proxy)",
        way_routed_prototype,
    ),
];

/// A new empty Array in prototype mode (an object has it as `[[Prototype]]`). An empty array
/// may have no index key, so making it a prototype pops nothing.
fn empty_array_prototype(rt: &Runtime) -> u64 {
    let p = object::new_array(rt, 0);
    let _child = object::new_object(rt, p);
    assert!(shape::is_prototype_object(p));
    assert!(!shape::proto_index_keys(rt), "an empty array prototype");
    p
}

fn way_sparse_element(cx: &mut crate::interp::Cx) {
    let rt = cx.rt();
    let p = empty_array_prototype(rt);
    // At the dense limit: the array moves to DICTIONARY (`array_set_sparse`).
    assert!(object::array_set(rt, p, object::arr::MAX_LENGTH, int(1)));
    assert_eq!(
        crate::elements::kind_of(p),
        crate::elements::kind::DICTIONARY
    );
}

fn way_element(cx: &mut crate::interp::Cx) {
    let rt = cx.rt();
    let p = empty_array_prototype(rt);
    assert!(object::array_set(rt, p, 0, int(1)));
    assert_eq!(object::array_get(p, 0), Some(int(1)));
}

fn way_dict_define(cx: &mut crate::interp::Cx) {
    let p = empty_array_prototype(cx.rt());
    // A read-only element: not {w, e, c}, so a DICTIONARY entry (`dict_define`).
    let k = key(cx.rt(), "0");
    let desc = crate::descriptor::Desc::data(int(1), attr::ENUMERABLE);
    assert!(property::define_own_property(cx, p, k, desc).unwrap());
    assert_eq!(
        crate::elements::kind_of(p),
        crate::elements::kind::DICTIONARY
    );
}

fn way_stored_key(cx: &mut crate::interp::Cx) {
    let rt = cx.rt();
    let p = object_with(rt, object_prototype(rt), &[]);
    let _child = object::new_object(rt, p);
    assert!(!shape::proto_index_keys(rt), "a plain prototype");
    // A raw store of a new index key: `props::own_add`.
    props::own_define(rt, p, key(rt, "3"), int(1), attr::DEFAULT);
}

fn way_becomes_prototype(cx: &mut crate::interp::Cx) {
    let rt = cx.rt();
    let a = object::new_array(rt, 0);
    assert!(object::array_set(rt, a, 0, int(1)));
    assert!(
        !shape::proto_index_keys(rt),
        "an element of a non-prototype"
    );
    let _child = object::new_object(rt, a);
}

fn way_routed_prototype(cx: &mut crate::interp::Cx) {
    let target = object::new_plain_object(cx.rt());
    let handler = object::new_plain_object(cx.rt());
    let p = proxy::proxy_create(cx, target, handler).expect("proxy_create");
    let rt = cx.rt();
    assert!(!shape::proto_index_keys(rt), "a proxy that is no prototype");
    let o = object::new_plain_object(rt);
    assert!(property::set_prototype_of(rt, o, p));
}

/// MC-w25-11 (V8 4635ddd858: one write path that did not pop its protector): each way an index
/// key reaches a prototype pops "no-elements prototypes", in realm 1 or in realm 2, and the
/// pop shows in every realm: both blocks read 0 at entry 6 (the other entries keep their
/// value), `shape::proto_index_keys` is true in each realm, a dependent is Invalidated, a new
/// one is refused, and a realm made after the pop starts popped.
#[test]
fn each_way_an_index_key_reaches_a_prototype_pops_the_fuse_in_every_realm() {
    for (name, way) in WAYS {
        for in_second in [false, true] {
            for stress in [false, true] {
                on_big_stack(move || {
                    let mut iso = isolate(config(Dispatch::DEFAULT, stress));
                    let r = iso.with_cx(|cx| {
                        let g = cx.create_realm()?;
                        let first = cx.inner().realm;
                        let rt: &Runtime = cx.inner().rt();
                        let second = realm::of_global(rt, g.bits()).unwrap();
                        let code = CodeHandle::new();
                        let f = FuseRef::Static(NO_ELEMENTS_PROTOTYPES);
                        rt.fuses.depend(rt, f, NULL, &code).unwrap();
                        for r in [first, second] {
                            assert_eq!(block_words(block_of(rt, r)), fresh_words(), "{name}");
                        }
                        if in_second {
                            cx.with_realm(g, |cx| {
                                way(cx.inner());
                                Ok(())
                            })?;
                        } else {
                            way(cx.inner());
                        }
                        let rt: &Runtime = cx.inner().rt();
                        for r in [first, second] {
                            assert_eq!(
                                block_words(block_of(rt, r)),
                                popped_words(),
                                "{name} (in realm {}): popped in every realm",
                                if in_second { 2 } else { 1 }
                            );
                        }
                        assert!(shape::proto_index_keys(rt), "{name}: realm 1 reads it");
                        cx.with_realm(g, |cx| {
                            let rt: &Runtime = cx.inner().rt();
                            assert!(shape::proto_index_keys(rt), "{name}: realm 2 reads it");
                            Ok(())
                        })?;
                        let rt: &Runtime = cx.inner().rt();
                        assert!(code.is_invalidated(), "{name}");
                        assert!(!rt.fuses.intact(rt, f, NULL));
                        assert_eq!(
                            rt.fuses.depend(rt, f, NULL, &CodeHandle::new()),
                            Err(Refusal::Popped)
                        );
                        let g3 = cx.create_realm()?;
                        let rt: &Runtime = cx.inner().rt();
                        let third = realm::of_global(rt, g3.bits()).unwrap();
                        assert_eq!(block_words(block_of(rt, third)), popped_words(), "{name}");
                        Ok(())
                    });
                    assert_eq!(r, Ok(()));
                    iso.collect_garbage();
                    iso.verify_heap().unwrap();
                });
            }
        }
    }
}

thread_local! {
    /// The calls of [`index_setter`] on this thread.
    static SETTER_CALLS: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

/// A setter host function: counts its calls.
fn index_setter<'s>(
    _cx: &mut crate::Cx<'s>,
    _this: crate::Local<'s>,
    _args: crate::Args<'s>,
    _nt: crate::Local<'s>,
) -> crate::JsResult<crate::Local<'s>> {
    SETTER_CALLS.with(|c| c.set(c.get() + 1));
    Ok(crate::Local::undefined())
}

/// The stale-read case of a per-realm pop (plan v11 §13 R19, OI R7): a prototype `p` of one
/// realm gets a setter at index 0 (`props::own_add` pops the fuse there); an Array of the
/// other realm has `p` as `[[Prototype]]`. `arr[0] = 5` in the other realm must walk to the
/// inherited setter (§10.1.9.2), so the other realm's word must read popped. With a pop that
/// wrote only the current realm's block, the other realm adds the element and the setter
/// never runs.
#[test]
fn a_cross_realm_prototype_chain_sees_the_pop_of_the_other_realm() {
    for pop_in_second in [false, true] {
        for stress in [false, true] {
            on_big_stack(move || {
                SETTER_CALLS.with(|c| c.set(0));
                let mut iso = isolate(config(Dispatch::DEFAULT, stress));
                let r = iso.with_cx(|cx| {
                    let g = cx.create_realm()?;
                    // The prototype with the setter, in the popping realm.
                    let make = |cx: &mut crate::interp::Cx| {
                        let rt = cx.rt();
                        let p = object::new_plain_object(rt);
                        let _child = object::new_object(rt, p);
                        assert!(!shape::proto_index_keys(rt));
                        let f = realm::new_host_function(rt, index_setter, "set0", 1, false);
                        let desc = crate::descriptor::Desc {
                            set: Some(f),
                            enumerable: Some(true),
                            configurable: Some(true),
                            ..Default::default()
                        };
                        let k = key(rt, "0");
                        assert!(property::define_own_property(cx, p, k, desc).unwrap());
                        p
                    };
                    // `arr[0] = 5` on an Array of the reading realm whose prototype is `p`.
                    let store = |cx: &mut crate::interp::Cx, p: u64| -> crate::conv::Res<u64> {
                        let rt = cx.rt();
                        assert!(
                            shape::proto_index_keys(rt),
                            "the reading realm sees the pop"
                        );
                        let arr = object::new_array(rt, 0);
                        assert!(property::set_prototype_of(rt, arr, p));
                        property::set_keyed(cx, arr, int(0), int(5), true)?;
                        Ok(arr)
                    };
                    let arr = if pop_in_second {
                        let p = cx.with_realm(g, |cx| Ok(make(cx.inner())))?;
                        store(cx.inner(), p)
                    } else {
                        let p = make(cx.inner());
                        cx.with_realm(g, |cx| Ok(store(cx.inner(), p)))?
                    };
                    let arr = arr.expect("the setter returns normally");
                    assert_eq!(
                        SETTER_CALLS.with(|c| c.get()),
                        1,
                        "the inherited setter ran"
                    );
                    assert_eq!(object::array_get(arr, 0), None, "no own element");
                    assert_eq!(object::array_len(arr), 0);
                    Ok(())
                });
                assert_eq!(r, Ok(()));
                iso.collect_garbage();
                iso.verify_heap().unwrap();
            });
        }
    }
}

/// The registry holds each realm block weakly: a realm that nothing holds dies with its block,
/// and the sweep drops the handle; a later pop writes the live blocks only.
#[test]
fn a_dead_realms_block_is_dropped_by_the_sweep() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut c = config(Dispatch::DEFAULT, stress);
            c.heap.record_conservative_roots = true;
            let mut iso = isolate(c);
            let masked = make_a_realm_and_drop_it(&mut iso);
            scrub_stack();
            iso.collect_garbage();
            let rt = &*iso.rt;
            let css = rt.heap.last_conservative_roots();
            assert!(
                !css.iter().any(|c| c.addr() == masked ^ MASK),
                "the conservative scan kept the dead block: the check proves nothing"
            );
            assert_eq!(rt.fuses.block_count(), 2, "not swept yet");
            rt.fuses.sweep(rt);
            assert_eq!(
                rt.fuses.block_count(),
                1,
                "the dead realm's block is dropped"
            );
            rt.fuses.pop(rt, FuseRef::Static(NO_ELEMENTS_PROTOTYPES));
            assert_eq!(block_words(rt.root(Root::RealmFuses)), popped_words());
            iso.verify_heap().unwrap();
        });
    }
}

/// Masks a cell address so the test's frame never holds it (the conservative scan, H10).
const MASK: usize = 0x5a5a_5a5a_5a5a_5a5a;

/// Overwrites 256 KiB of stack below the caller (plan v10 rule 35), so stale words of returned
/// frames hold no cell for the conservative scan.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    std::hint::black_box(&mut words);
}

/// Makes a second realm in its own `with_cx` (its handle scope ends there) and gives the
/// masked address of its block; out of line, so no frame of the caller holds it.
#[inline(never)]
fn make_a_realm_and_drop_it(iso: &mut Isolate) -> usize {
    let r = iso.with_cx(|cx| {
        let g = cx.create_realm()?;
        let rt: &Runtime = cx.inner().rt();
        let second = realm::of_global(rt, g.bits()).unwrap();
        let b = block_of(rt, second);
        assert_eq!(rt.fuses.block_count(), 2);
        Ok(std::hint::black_box(b as usize) ^ MASK)
    });
    r.unwrap()
}

/// The one writer (MC-w25-11, V8 4635ddd858): only `shape::note_proto_index_key` pops the
/// fuse, and it has exactly the 5 callers of [`WAYS`]. A new path that gives a prototype an
/// index key must call it and add a test above; this list then fails until it is updated.
#[test]
fn note_proto_index_key_is_the_one_writer() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut pops = Vec::new();
    let mut notes = Vec::new();
    let mut stack = vec![src.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if !p.ends_with("tests") {
                    stack.push(p);
                }
                continue;
            }
            if p.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let rel = p.strip_prefix(&src).unwrap().display().to_string();
            for line in std::fs::read_to_string(&p).unwrap().lines() {
                let code = line.split("//").next().unwrap();
                if code.contains("pop_no_elements(") && !code.contains("fn pop_no_elements(") {
                    pops.push(rel.clone());
                }
                if code.contains("Static(NO_ELEMENTS_PROTOTYPES)") {
                    pops.push(format!("{rel} (Static)"));
                }
                if code.contains("note_proto_index_key(") && !code.contains("fn note_") {
                    notes.push(rel.clone());
                }
            }
        }
    }
    pops.sort();
    notes.sort();
    assert_eq!(pops, ["fuse.rs (Static)", "shape.rs"], "the pop sites");
    assert_eq!(
        notes,
        [
            "object.rs",
            "object.rs",
            "property.rs",
            "props.rs",
            "shape.rs"
        ],
        "the callers of note_proto_index_key"
    );
}
