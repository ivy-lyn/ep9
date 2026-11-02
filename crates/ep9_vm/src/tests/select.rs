//! Tests of `select()` (`select.rs`) (owner M6-P0 MODES6; doc 13 C-15, MODES-D13,
//! MODES-D14): the T1-T11 table of doc 13 §3.3 with fake code objects, one negative test per
//! append-only list that `select()` reads (probe at the top of the range, never the next free
//! value, doc 13 §5.4), JIT and hybrid tier-up, and PIN with two Active AotOpt variants.
//!
//! ```text
//!  event (T1-T11) -> change the fake code set / session -> select() under JIT, hybrid, jitless
//! ```

use crate::dispatch::DispatchKind::{self, AotBaseline, AotOpt, Baseline, Interp, Opt};
use crate::exec::Preset;
use crate::fuse::{STATIC, descriptor};
use crate::select::{CodeObject, CodeSet, Dep, Pick, Policy, Session, State, lowering, select};

const ALL: [(&str, Policy); 3] = [
    ("jit", Policy::JIT),
    ("hybrid", Policy::HYBRID),
    ("jitless", Policy::JITLESS),
];

fn obj(kind: DispatchKind, low: u8) -> CodeObject {
    CodeObject::new(kind, low)
}

fn set(objects: Vec<CodeObject>) -> CodeSet {
    CodeSet {
        objects,
        ..CodeSet::default()
    }
}

fn pick(s: &CodeSet, p: Policy) -> Pick {
    select(s, &p, &Session::default())
}

/// The kind of the picked object, or `Interp` for an Interp variant.
fn kind_of(s: &CodeSet, p: Pick) -> DispatchKind {
    match p {
        Pick::Code(i) => DispatchKind::from_u8(s.objects[i].kind).unwrap(),
        Pick::Interp | Pick::InterpDebug => Interp,
    }
}

#[test]
fn an_empty_code_set_runs_bytecode_under_every_preset() {
    for (name, p) in ALL {
        assert_eq!(pick(&CodeSet::default(), p), Pick::Interp, "{name}");
    }
}

#[test]
fn the_policies_are_the_doc_13_presets() {
    // Doc 13 §1.2: install rank and accept list, in rank order (MODES-D13).
    assert_eq!(Policy::JIT.install, &[Opt, Baseline, Interp]);
    assert_eq!(Policy::JIT.accept, &[lowering::JIT]);
    assert_eq!(Policy::JITLESS.install, &[AotOpt, AotBaseline, Interp]);
    assert_eq!(Policy::JITLESS.accept, &[lowering::JITLESS]);
    assert_eq!(
        Policy::HYBRID.install,
        &[Opt, Baseline, AotOpt, AotBaseline, Interp]
    );
    assert_eq!(
        Policy::HYBRID.accept,
        &[lowering::JIT, lowering::HYBRID, lowering::JITLESS]
    );
    // Every preset that runs a runtime producer lists `jit` (MODES-D13).
    for p in [Policy::JIT, Policy::HYBRID] {
        assert!(p.accept.contains(&lowering::JIT));
    }
    assert_eq!(Policy::of(Preset::JIT), Some(Policy::JIT));
    assert_eq!(Policy::of(Preset::JITLESS), Some(Policy::JITLESS));
}

// ---- T1-T11 (doc 13 §3.3) ----

#[test]
fn t1_unit_load_adds_aot_objects_that_hybrid_and_jitless_pick() {
    // A jitless unit: AotOpt and AotBaseline, both lowering jitless.
    let s = set(vec![
        obj(AotBaseline, lowering::JITLESS),
        obj(AotOpt, lowering::JITLESS),
    ]);
    assert_eq!(pick(&s, Policy::JITLESS), Pick::Code(1));
    assert_eq!(pick(&s, Policy::HYBRID), Pick::Code(1));
    // JIT: n/a. Its rank has no AOT kind, so unit code never runs there.
    assert_eq!(pick(&s, Policy::JIT), Pick::Interp);
    // A hybrid unit variant: hybrid takes it; jitless skips it (not in its accept list).
    let h = set(vec![obj(AotOpt, lowering::HYBRID)]);
    assert_eq!(pick(&h, Policy::HYBRID), Pick::Code(0));
    assert_eq!(pick(&h, Policy::JITLESS), Pick::Interp);
}

#[test]
fn t1_a_pinned_function_skips_aot_opt_objects_a_later_unit_load_adds() {
    let mut s = set(vec![obj(AotBaseline, lowering::JITLESS)]);
    s.pinned = true;
    // The unit load comes after PIN (MODES-D14).
    s.objects.push(obj(AotOpt, lowering::JITLESS));
    s.objects.push(obj(AotOpt, lowering::HYBRID));
    assert_eq!(pick(&s, Policy::JITLESS), Pick::Code(0));
    assert_eq!(pick(&s, Policy::HYBRID), Pick::Code(0));
}

#[test]
fn t2_jit_and_hybrid_tier_up_picks_jit_code_and_jitless_skips_it() {
    // C-15: a `jit` Baseline or Opt object is picked under JIT and hybrid, skipped under
    // jitless (MODES-D13).
    let mut s = CodeSet::default();
    s.objects.push(obj(Baseline, lowering::JIT));
    assert_eq!(pick(&s, Policy::JIT), Pick::Code(0));
    assert_eq!(pick(&s, Policy::HYBRID), Pick::Code(0));
    assert_eq!(pick(&s, Policy::JITLESS), Pick::Interp);
    s.objects.push(obj(Opt, lowering::JIT));
    assert_eq!(pick(&s, Policy::JIT), Pick::Code(1));
    assert_eq!(pick(&s, Policy::HYBRID), Pick::Code(1));
    assert_eq!(pick(&s, Policy::JITLESS), Pick::Interp);
}

#[test]
fn t2_jit_code_beats_aot_code_in_hybrid_and_loses_to_it_in_jitless() {
    let s = set(vec![
        obj(AotOpt, lowering::JITLESS),
        obj(Opt, lowering::JIT),
    ]);
    assert_eq!(kind_of(&s, pick(&s, Policy::HYBRID)), Opt);
    assert_eq!(kind_of(&s, pick(&s, Policy::JITLESS)), AotOpt);
    assert_eq!(kind_of(&s, pick(&s, Policy::JIT)), Opt);
}

#[test]
fn t3_hot_aot_opt_retiers_through_baseline_in_hybrid_only() {
    let mut s = set(vec![obj(AotOpt, lowering::HYBRID)]);
    assert_eq!(pick(&s, Policy::HYBRID), Pick::Code(0));
    // The budget counter fires: T1 makes instrumented Baseline code (lowering jit). The
    // hybrid rank puts Baseline above AotOpt on purpose (doc 13 §1.2).
    s.objects.push(obj(Baseline, lowering::JIT));
    assert_eq!(pick(&s, Policy::HYBRID), Pick::Code(1));
    // Then T2.
    s.objects.push(obj(Opt, lowering::JIT));
    assert_eq!(pick(&s, Policy::HYBRID), Pick::Code(2));
    // jitless never re-tiers; had it the objects, it would skip them.
    let j = set(vec![
        obj(AotOpt, lowering::JITLESS),
        obj(Baseline, lowering::JIT),
    ]);
    assert_eq!(pick(&j, Policy::JITLESS), Pick::Code(0));
}

#[test]
fn t4_pin_with_two_active_aot_opt_variants_picks_neither() {
    // MODES-D14: two Active AotOpt variants (two CPU levels), then PIN.
    let mut lo = obj(AotOpt, lowering::JITLESS);
    lo.cpu = 0;
    let mut hi = obj(AotOpt, lowering::JITLESS);
    hi.cpu = 0b10;
    let host = Session {
        cpu: 0b10,
        ..Session::default()
    };
    let mut s = set(vec![hi, lo, obj(AotBaseline, lowering::JITLESS)]);
    assert_eq!(select(&s, &Policy::JITLESS, &host), Pick::Code(0));
    s.pinned = true;
    // jitless: AotBaseline (H32:42-43).
    assert_eq!(select(&s, &Policy::JITLESS, &host), Pick::Code(2));
    // Both variants stay Active: PIN is a function state, not an object state.
    assert!(s.objects.iter().all(|o| o.state == State::Active));
    // Without an AotBaseline, Interp.
    s.objects.pop();
    assert_eq!(select(&s, &Policy::JITLESS, &host), Pick::Interp);
    // An AotOpt object installed after PIN is skipped too.
    s.objects.push(obj(AotOpt, lowering::JITLESS));
    assert_eq!(select(&s, &Policy::JITLESS, &host), Pick::Interp);
}

#[test]
fn t4_pin_in_hybrid_holds_until_the_retier_gives_jit_code() {
    let mut s = set(vec![
        obj(AotOpt, lowering::HYBRID),
        obj(AotOpt, lowering::JITLESS),
    ]);
    assert_eq!(pick(&s, Policy::HYBRID), Pick::Code(0));
    s.pinned = true;
    assert_eq!(pick(&s, Policy::HYBRID), Pick::Interp);
    // The re-tier starts (H33-Q4): its jit Baseline is not AotOpt, so PIN does not hide it.
    s.objects.push(obj(Baseline, lowering::JIT));
    assert_eq!(pick(&s, Policy::HYBRID), Pick::Code(2));
}

#[test]
fn t5_t8_t9_events_with_no_slot_write_keep_the_pick() {
    // T5 (eager deopt: the frame goes to T0, the code stays), T8 (OSR out of AOT code: a
    // lazy deopt at the poll) and T9 (reserved) change no code set, so select() gives the
    // same answer: the slot needs no write.
    for (name, p) in ALL {
        let s = set(vec![
            obj(Opt, lowering::JIT),
            obj(AotOpt, lowering::HYBRID),
            obj(AotOpt, lowering::JITLESS),
        ]);
        // select() is pure: the same inputs give the same pick.
        let before = pick(&s, p);
        assert_eq!(pick(&s, p), before, "{name}");
        assert_ne!(before, Pick::Interp, "{name}");
    }
}

#[test]
fn t6_a_fuse_pop_skips_dependents_and_keeps_guard_fuse_code() {
    let no_elements = STATIC
        .iter()
        .position(|&n| n == "no-elements prototypes")
        .unwrap();
    let dep = |intact| Dep::Static {
        index: no_elements as u32,
        intact,
    };
    // JIT and hybrid: the dependent is Invalidated and its dependency popped.
    for p in [Policy::JIT, Policy::HYBRID] {
        let mut o = obj(Opt, lowering::JIT);
        o.deps.push(dep(true));
        let mut s = set(vec![obj(Baseline, lowering::JIT), o]);
        assert_eq!(pick(&s, p), Pick::Code(1));
        // The pop, before the registry marks the object: the dependency already fails.
        s.objects[1].deps[0] = dep(false);
        assert_eq!(pick(&s, p), Pick::Code(0));
        // Then Invalidated (M-R5: never an object that is not Active).
        s.objects[1].state = State::Invalidated;
        s.objects[1].deps[0] = dep(true);
        assert_eq!(pick(&s, p), Pick::Code(0));
    }
    // jitless: GuardFuse code reads the fuse and takes the slow path; it has no dependency,
    // so it stays picked after the pop. InvalidatingFuse code goes as in JIT.
    let mut inv = obj(AotOpt, lowering::JITLESS);
    inv.deps.push(Dep::Dynamic {
        kind: descriptor::PROTO_CHAIN,
        intact: true,
    });
    let s = set(vec![inv.clone(), obj(AotBaseline, lowering::JITLESS)]);
    assert_eq!(pick(&s, Policy::JITLESS), Pick::Code(0));
    let mut popped = s.clone();
    popped.objects[0].deps[0] = Dep::Dynamic {
        kind: descriptor::PROTO_CHAIN,
        intact: false,
    };
    assert_eq!(pick(&popped, Policy::JITLESS), Pick::Code(1));
    let guard = set(vec![obj(AotOpt, lowering::JITLESS)]);
    assert_eq!(pick(&guard, Policy::JITLESS), Pick::Code(0));
}

#[test]
fn t7_osr_into_optimized_code_installs_opt() {
    for p in [Policy::JIT, Policy::HYBRID] {
        let mut s = set(vec![obj(Baseline, lowering::JIT)]);
        s.objects.push(obj(Opt, lowering::JIT));
        assert_eq!(pick(&s, p), Pick::Code(1));
    }
}

#[test]
fn t10_debugger_attach_invalidates_jit_code_and_keeps_aot_code() {
    let connected = Session {
        connected: true,
        ..Session::default()
    };
    // JIT code: InspectorSession is an InvalidatingFuse, so the code is Invalidated.
    let inspector = STATIC
        .iter()
        .position(|&n| n == "InspectorSession")
        .unwrap() as u32;
    let mut o = obj(Opt, lowering::JIT);
    o.deps.push(Dep::Static {
        index: inspector,
        intact: false,
    });
    o.state = State::Invalidated;
    let mut s = set(vec![o]);
    s.debug_copy = true;
    assert_eq!(select(&s, &Policy::JIT, &connected), Pick::InterpDebug);
    s.debug_copy = false;
    assert_eq!(select(&s, &Policy::JIT, &connected), Pick::Interp);
    // AOT code reads InspectorSession as a GuardFuse: it stays Active and picked (H24:22).
    let a = set(vec![obj(AotOpt, lowering::JITLESS)]);
    assert_eq!(select(&a, &Policy::JITLESS, &connected), Pick::Code(0));
    assert_eq!(select(&a, &Policy::HYBRID, &connected), Pick::Code(0));
}

#[test]
fn t11_a_breakpoint_picks_the_interp_debug_copy_and_clearing_it_drops_it() {
    let connected = Session {
        connected: true,
        ..Session::default()
    };
    for (name, p) in ALL {
        let mut s = set(vec![
            obj(Opt, lowering::JIT),
            obj(AotOpt, lowering::JITLESS),
        ]);
        s.debug_copy = true;
        let normal = select(&s, &p, &connected);
        s.breakpoint = true;
        assert_eq!(select(&s, &p, &connected), Pick::InterpDebug, "{name}");
        // A breakpoint without a session changes nothing.
        assert_eq!(select(&s, &p, &Session::default()), normal, "{name}");
        s.breakpoint = false;
        assert_eq!(select(&s, &p, &connected), normal, "{name}");
    }
}

// ---- rank order and filters ----

#[test]
fn the_accept_rank_orders_lowering_classes_inside_one_kind() {
    // Hybrid accepts jit, hybrid, jitless in that order: a hybrid AotOpt variant beats a
    // jitless one, whatever their order in the set.
    let s = set(vec![
        obj(AotOpt, lowering::JITLESS),
        obj(AotOpt, lowering::HYBRID),
    ]);
    assert_eq!(pick(&s, Policy::HYBRID), Pick::Code(1));
    assert_eq!(pick(&s, Policy::JITLESS), Pick::Code(0));
}

#[test]
fn a_cpu_level_the_host_lacks_is_skipped() {
    let mut fast = obj(AotOpt, lowering::JITLESS);
    fast.cpu = 1 << 63;
    let s = set(vec![fast, obj(AotOpt, lowering::JITLESS)]);
    assert_eq!(pick(&s, Policy::JITLESS), Pick::Code(1));
    let host = Session {
        cpu: 1 << 63,
        ..Session::default()
    };
    assert_eq!(select(&s, &Policy::JITLESS, &host), Pick::Code(0));
}

#[test]
fn an_object_that_is_not_active_is_never_picked() {
    // M-R5, every kind and preset.
    for (name, p) in ALL {
        for k in DispatchKind::ALL {
            for l in [lowering::JIT, lowering::HYBRID, lowering::JITLESS] {
                let mut o = obj(k, l);
                o.state = State::Invalidated;
                assert_eq!(pick(&set(vec![o]), p), Pick::Interp, "{name} {k:?} {l}");
            }
        }
    }
}

// ---- append-only lists: a probe at the top of the range (doc 13 §5.4) ----

#[test]
fn an_unknown_dispatch_kind_byte_is_never_picked() {
    // H14 dispatch kind bytes (C-03): the high byte.
    assert_eq!(DispatchKind::from_u8(0xFE), None);
    for (name, p) in ALL {
        for l in [lowering::JIT, lowering::HYBRID, lowering::JITLESS] {
            let mut o = obj(Opt, l);
            o.kind = 0xFE;
            assert_eq!(pick(&set(vec![o]), p), Pick::Interp, "{name} {l}");
        }
    }
}

#[test]
fn an_unknown_lowering_class_is_never_picked() {
    // H32 lowering classes (C-07): class 0xFE.
    assert!(!lowering::known(0xFE));
    for (name, p) in ALL {
        assert!(!p.accept.contains(&0xFE), "{name}");
        for k in DispatchKind::ALL {
            let o = obj(k, 0xFE);
            assert_eq!(pick(&set(vec![o]), p), Pick::Interp, "{name} {k:?}");
        }
    }
}

#[test]
fn an_unknown_static_fuse_index_skips_the_object() {
    // H31 static fuse list: the top index.
    let mut o = obj(Opt, lowering::JIT);
    o.deps.push(Dep::Static {
        index: u32::MAX,
        intact: true,
    });
    assert!(!o.deps[0].holds());
    assert_eq!(pick(&set(vec![o]), Policy::JIT), Pick::Interp);
    // Every known index that is intact holds.
    for i in 0..STATIC.len() as u32 {
        assert!(
            Dep::Static {
                index: i,
                intact: true
            }
            .holds()
        );
    }
}

#[test]
fn an_unknown_fuse_descriptor_kind_skips_the_object() {
    // H31 dynamic descriptor kinds (C-01): kind 0xFE.
    let mut o = obj(AotOpt, lowering::JITLESS);
    o.deps.push(Dep::Dynamic {
        kind: 0xFE,
        intact: true,
    });
    assert!(!o.deps[0].holds());
    assert_eq!(pick(&set(vec![o]), Policy::JITLESS), Pick::Interp);
    for k in [
        descriptor::PROTO_CHAIN,
        descriptor::CONSTRUCTOR_CHAIN,
        descriptor::FIELD,
    ] {
        assert!(
            Dep::Dynamic {
                kind: k,
                intact: true
            }
            .holds()
        );
    }
}
