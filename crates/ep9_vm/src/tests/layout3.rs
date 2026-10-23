//! OBJECT_LAYOUT 3 (M3-P0, owner P0-VM3): the FUNCTION `[[Realm]]` field and the realm
//! record, `Cx::realm`, the new kinds (ARGUMENTS, MODULE_NAMESPACE, MODULE, ORDERED_TABLE,
//! REALM) and their trace functions, and the routing of the exotic hooks, of the
//! module-variable and arguments opcodes, and of the ABI 5 runtime ids 32-43, 46 and 47 to
//! their month-3 files (plan V3-A5, V3-A13, P0-VM3 task 8).

use std::hint::black_box;

use ep9_bytecode::{FunctionFlags, FunctionId, RegisterList, RuntimeId, VerifiedUnit};
use ep9_heap::{CellKind, CellSize, GlobalHandle, KindError, KindId};

use super::{Asm, Run, config, isolate, meta, on_big_stack, run_in, take_output, unit};
use crate::builtins::{self, BuiltinId, IntrinsicId, intrinsic};
use crate::descriptor::Desc;
use crate::exotic::probe;
use crate::frame::{NULL, UNDEFINED, int32_bits};
use crate::object::{self, argsf, attr, cell, fun, modf, nsf, obj, realmf};
use crate::ordered_table::{self, MAP_WIDTH, SET_WIDTH, otf};
use crate::runtime::{Atom, Root, Runtime, val};
use crate::{Dispatch, Isolate, arguments, kinds, module, property, props, shape, versions};

// ---- versions, kinds, fixed fields ------------------------------------------------------------

#[test]
fn layout_3_versions_kinds_and_fixed_fields() {
    assert_eq!(versions::OBJECT_LAYOUT_VERSION, 3);
    // The other contract versions do not change in M3-P0.
    assert_eq!(
        (
            versions::FRAME_VERSION,
            versions::IC_LAYOUT_VERSION,
            versions::BUILTIN_ABI_VERSION,
            versions::DEOPT_META_VERSION
        ),
        (2, 1, 1, 0)
    );
    let ids = [
        kinds::ARGUMENTS,
        kinds::MODULE_NAMESPACE,
        kinds::MODULE,
        kinds::ORDERED_TABLE,
        kinds::REALM,
    ];
    assert_eq!(ids.map(KindId::get), [25, 26, 76, 77, 78]);
    // FUNCTION: +48 realm, so the in-object slots start at +56.
    assert_eq!(
        (fun::INFO, fun::CONTEXT, fun::HOME, fun::REALM),
        (24, 32, 40, 48)
    );
    assert_eq!(fun::FIXED, 4);
    assert_eq!(object::fixed_count(kinds::FUNCTION.get()), 4);
    assert_eq!(object::fixed_count(kinds::ARGUMENTS.get()), argsf::FIXED);
    assert_eq!(
        object::fixed_count(kinds::MODULE_NAMESPACE.get()),
        nsf::FIXED
    );
    assert_eq!(
        (
            argsf::MAP,
            argsf::MAPPED_COUNT,
            argsf::FIXED,
            argsf::CAPACITY
        ),
        (24, 32, 2, 3)
    );
    assert_eq!(
        (nsf::MODULE, nsf::EXPORTS, nsf::FIXED, nsf::CAPACITY),
        (24, 32, 2, 1)
    );
    // MODULE: 16 Values after word0; REALM: 5.
    assert_eq!(modf::SIZE, 8 + 8 * 16);
    assert_eq!((modf::STATUS, modf::HOST_DEFINED), (8, 128));
    assert_eq!(realmf::SIZE, 8 + 8 * 5);
    assert_eq!(
        (
            realmf::GLOBAL_OBJECT,
            realmf::GLOBAL_LEXICAL,
            realmf::INTRINSICS,
            realmf::TEMPLATES,
            realmf::HOST_DEFINED
        ),
        (8, 16, 24, 32, 40)
    );
    assert_eq!(
        (
            otf::CAPACITY,
            otf::LIVE,
            otf::DELETED,
            otf::BUCKETS,
            otf::WIDTH,
            otf::FLAGS,
            otf::DATA
        ),
        (8, 16, 24, 32, 40, 48, 56)
    );
}

/// `object::register_kinds` registers the 5 new kinds (15 entries of its own array, plus the
/// `ordered_table::register_kinds` call): a second registration of each is refused. A reserved
/// id (27) is still free.
#[test]
fn register_kinds_registers_the_five_new_kinds() {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    fn dummy(_: ep9_heap::CellPtr, _: &mut ep9_heap::Tracer<'_>) {}
    let k = |name| CellKind {
        name,
        size: CellSize::Fixed(16),
        trace: dummy,
    };
    for id in [
        kinds::ARGUMENTS,
        kinds::MODULE_NAMESPACE,
        kinds::MODULE,
        kinds::ORDERED_TABLE,
        kinds::REALM,
    ] {
        assert_eq!(
            iso.rt.heap.register_kind(id, k("again")),
            Err(KindError::Taken(id)),
            "kind {} is registered",
            id.get()
        );
    }
    let free = KindId::new(27).unwrap();
    assert_eq!(iso.rt.heap.register_kind(free, k("free")), Ok(()));
}

// ---- the realm --------------------------------------------------------------------------------

/// f1: `function f(a, b) {}`; f2: `function* g() {}`. The script stores both as globals.
fn two_functions() -> VerifiedUnit {
    let mut f = Asm::new(meta("f", 2, 0, 0, 0));
    f.b.lda_undefined().return_();
    let mut gm = meta("g", 0, 0, 0, 0);
    gm.flags = FunctionFlags::SIMPLE_PARAMETERS | FunctionFlags::GENERATOR;
    let mut g = Asm::new(gm);
    g.b.lda_undefined().return_();
    let mut s = Asm::script(0);
    for (i, name) in ["f", "g"].iter().enumerate() {
        let k = s.b.function_constant(FunctionId(i as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        s.sta_global(name);
    }
    s.b.lda_undefined().return_();
    unit("", vec![s.finish(), f.finish(), g.finish()])
}

/// Checks that the function `f` has the isolate's REALM cell as its `[[Realm]]`.
fn assert_realm(rt: &Runtime, f: u64, what: &str) {
    assert!(kinds::is_function(f), "{what} is a function");
    let realm = object::function_realm(f);
    assert_eq!(
        realm,
        rt.root(Root::Realm),
        "{what}: [[Realm]] is the isolate's realm"
    );
}

/// Plan V3-A5: every function (bytecode, generator, host, builtin, a bound function's
/// target) has the isolate's REALM cell at +48; `Cx::realm` equals the REALM root; the
/// record mirrors the roots and the intrinsics; the in-object slots moved to +56.
#[test]
fn every_function_has_the_isolate_realm() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            iso.run_script(two_functions(), "realm.js").unwrap();
            take_output();
            let rt: &Runtime = &iso.rt;
            let realm = rt.root(Root::Realm);
            assert_eq!(kinds::cell_kind(realm), kinds::REALM.get());
            assert_eq!(iso.cx.realm, realm, "Cx::realm is a copy of the REALM root");
            let f = iso.global("f").unwrap().to_bits();
            let g = iso.global("g").unwrap().to_bits();
            assert!(object::function_info(g).is_generator());
            assert_realm(rt, f, "a bytecode function");
            assert_realm(rt, g, "a generator function");
            assert_realm(
                rt,
                iso.global("print").unwrap().to_bits(),
                "a host function",
            );
            // Every function of the VM table, %Function.prototype% (made before any other
            // function) included.
            let vm = &crate::natives::VM_TABLE.modules[0];
            for i in 0..vm.defs.len() {
                let b = builtins::function_object(rt, BuiltinId::new(vm.block, i as u16));
                assert_realm(rt, b, vm.defs[i].name);
            }
            assert_realm(rt, rt.root(Root::FunctionPrototype), "%Function.prototype%");
            // A bound function's target.
            let cx: &mut crate::interp::Cx = &mut iso.cx;
            let bf = crate::bound::new_bound_function(cx, f, UNDEFINED, &[]).unwrap();
            assert_realm(rt, crate::bound::target(bf), "a bound function's target");
            // The in-object slots of a function moved to +56: `length` 2, `name` "f".
            let c = cell(f);
            assert_eq!(obj::FIXED + 8 * fun::FIXED, 56);
            assert_eq!(val(object::rd(c, 56)).as_number(), Some(2.0));
            assert_eq!(object::rd(c, 64), crate::atoms::intern_str(rt, "f"));
            // The record mirrors the roots and the intrinsics.
            assert_eq!(
                object::record_get(realm, realmf::GLOBAL_OBJECT),
                rt.global()
            );
            assert_eq!(
                object::record_get(realm, realmf::GLOBAL_LEXICAL),
                rt.root(Root::Lexical)
            );
            assert_eq!(object::record_get(realm, realmf::TEMPLATES), UNDEFINED);
            assert_eq!(object::record_get(realm, realmf::HOST_DEFINED), UNDEFINED);
            let list = object::record_get(realm, realmf::INTRINSICS);
            assert_eq!(object::list_len(list), IntrinsicId::ALL.len());
            assert_eq!(IntrinsicId::ALL.len(), 64);
            for id in IntrinsicId::ALL {
                assert_eq!(
                    object::list_get(list, id as usize),
                    intrinsic(rt, id),
                    "{}",
                    id.name()
                );
            }
            // Ids 45-63 are declared, not made (P0-D3).
            for id in &IntrinsicId::ALL[45..] {
                assert_eq!(intrinsic(rt, *id), UNDEFINED, "{}", id.name());
            }
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

// ---- GC: every Value field of the new kinds is traced -------------------------------------------

const MASK: u64 = 0x5a5a_5a5a_5a5a_5a5a;

/// Bytes of zeros between the test's frame and the frames that make the cells (the pattern of
/// tests/generator.rs and tests/ic.rs; tests/mod.rs is frozen, so each file has its own copy).
const DEEP: usize = 64 << 10;

/// Overwrites 256 KiB of stack below the caller.
#[inline(never)]
fn scrub_stack() {
    let mut words = [0usize; 1 << 15];
    black_box(&mut words);
}

/// Runs `made` with [`DEEP`] bytes of zeros between its frames and the caller's.
#[inline(never)]
fn below_a_pad<R>(made: impl FnOnce() -> R) -> (R, usize) {
    let mut pad = [0u8; DEEP];
    black_box(&mut pad);
    let r = made();
    (r, black_box(&mut pad).as_ptr().addr())
}

/// True if a live cell starts at the masked address `masked`.
#[inline(never)]
fn is_live(rt: &Runtime, masked: u64) -> bool {
    rt.heap.is_cell(cell(black_box(masked) ^ MASK).addr())
}

/// One holder cell per new kind (and a FUNCTION for its moved slots), each rooted by a global
/// handle, with a fresh object in every Value field the test fills. Returns the handles and
/// the masked bits of the fresh objects, with the field they sit in.
#[inline(never)]
fn made_holders(rt: &Runtime) -> (Vec<GlobalHandle>, Vec<(String, u64)>) {
    let mut handles = Vec::new();
    let mut fields = Vec::new();
    let mut fill = |h: u64, what: &str, offs: &mut dyn Iterator<Item = usize>| {
        handles.push(rt.heap.new_global(val(h)));
        for off in offs {
            let o = object::new_plain_object(rt);
            object::wr(&rt.heap, cell(h), off, o);
            fields.push((format!("{what} +{off}"), black_box(o) ^ MASK));
        }
    };
    let proto = rt.root(Root::ObjectPrototype);
    // ARGUMENTS: 2 fixed fields and 3 in-object slots (+24 .. +64).
    let a = arguments::new_arguments_object(rt, proto);
    fill(a, "ARGUMENTS", &mut (24..64).step_by(8));
    // MODULE_NAMESPACE: 2 fixed fields and 1 in-object slot (+24 .. +48).
    let m = module::new_module_record(rt);
    let names = object::new_list(rt, 0, UNDEFINED);
    let ns = module::new_namespace(rt, m, names);
    fill(ns, "MODULE_NAMESPACE", &mut (24..48).step_by(8));
    // MODULE: 16 Values.
    let m = module::new_module_record(rt);
    fill(m, "MODULE", &mut (8..modf::SIZE).step_by(8));
    // ORDERED_TABLE: 2 buckets, 2 Map entries: one bucket head and every entry slot.
    let t = ordered_table::new_table(rt, 2, 2, MAP_WIDTH, false);
    fill(
        t,
        "ORDERED_TABLE",
        &mut (otf::DATA + 8..otf::DATA + 8 * (2 + 2 * 3)).step_by(8),
    );
    // REALM: 5 Values.
    let r = crate::realm::new_realm_record(rt);
    fill(r, "REALM", &mut (8..realmf::SIZE).step_by(8));
    // FUNCTION: [[HomeObject]] (+40) and the 2 in-object slots, which moved to +56 and +64.
    let f = crate::realm::new_host_function(rt, super::print, "h", 0, false);
    fill(f, "FUNCTION", &mut [40, 56, 64].into_iter());
    (handles, fields)
}

/// Plan V3-A5: a cell of each new kind survives a GC with every Value field traced. A field
/// that a trace function skipped would free its object (a break of one trace function, for
/// example `trace_realm` over 40 bytes, fails here).
#[test]
fn new_kinds_survive_a_gc_with_every_field_traced() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut c = config(Dispatch::DEFAULT, stress);
            c.heap.record_conservative_roots = true;
            let iso = isolate(c);
            let rt: &Runtime = &iso.rt;
            let ((handles, fields), pad_low) = below_a_pad(|| made_holders(rt));
            scrub_stack();
            rt.heap.collect();
            let high = ep9_platform::current_thread_stack_bounds().unwrap().high;
            let scan_low = high - 8 * rt.heap.stats().last_stack_words;
            assert!(scan_low > pad_low, "the scan stopped above the pad");
            let css = rt.heap.last_conservative_roots();
            for (what, m) in &fields {
                let addr = cell(m ^ MASK).addr();
                assert!(
                    !css.iter().any(|c| c.addr() == addr),
                    "{what}: the conservative scan kept it"
                );
            }
            for (what, m) in &fields {
                assert!(
                    below_a_pad(|| is_live(rt, *m)).0,
                    "{what} was traced (stress={stress})"
                );
            }
            iso.verify_heap().unwrap();
            for h in handles {
                rt.heap.drop_global(h);
            }
        });
    }
}

/// F2: a new MODULE record has no `[[EvaluationError]]`: the hole (H17: hole = none, since
/// `throw undefined` is a legal error); every other field is undefined. The hole survives a
/// collection (the tracer and the heap verifier accept it in a MODULE slot).
#[test]
fn a_new_module_record_has_no_evaluation_error() {
    for stress in [false, true] {
        on_big_stack(move || {
            let iso = isolate(config(Dispatch::DEFAULT, stress));
            let rt: &Runtime = &iso.rt;
            let m = module::new_module_record(rt);
            assert_eq!(
                object::record_get(m, modf::EVALUATION_ERROR),
                crate::frame::HOLE
            );
            for off in (8..modf::SIZE).step_by(8) {
                if off != modf::EVALUATION_ERROR {
                    assert_eq!(object::record_get(m, off), UNDEFINED, "+{off}");
                }
            }
            let h = rt.heap.new_global(val(m));
            rt.heap.collect();
            let m = rt.heap.global(h).to_bits();
            assert_eq!(
                object::record_get(m, modf::EVALUATION_ERROR),
                crate::frame::HOLE,
                "after a collection"
            );
            iso.verify_heap().unwrap();
            rt.heap.drop_global(h);
        });
    }
}

/// An ORDERED_TABLE made with the weak flag keeps it (through GCs); its size is read from the
/// cell: 56 + 8 x (buckets + capacity x width).
#[test]
fn an_ordered_table_keeps_its_weak_flag_and_size() {
    for stress in [false, true] {
        on_big_stack(move || {
            let iso = isolate(config(Dispatch::DEFAULT, stress));
            let rt: &Runtime = &iso.rt;
            let weak = ordered_table::new_table(rt, 4, 8, SET_WIDTH, true);
            let hw = rt.heap.new_global(val(weak));
            let strong = ordered_table::new_table(rt, 3, 4, MAP_WIDTH, false);
            let hs = rt.heap.new_global(val(strong));
            for _ in 0..3 {
                rt.heap.collect();
            }
            let (weak, strong) = (rt.heap.global(hw).to_bits(), rt.heap.global(hs).to_bits());
            assert!(ordered_table::is_weak(weak));
            assert!(!ordered_table::is_weak(strong));
            assert_eq!(ordered_table::size_of_table(weak), 56 + 8 * (8 + 4 * 2));
            assert_eq!(ordered_table::size_of_table(strong), 56 + 8 * (4 + 3 * 3));
            assert_eq!(
                object::rd(cell(weak), otf::FLAGS),
                int32_bits(ordered_table::FLAG_WEAK_KEYS)
            );
            assert_eq!(
                object::rd(cell(strong), otf::DATA),
                int32_bits(-1),
                "empty bucket"
            );
            iso.verify_heap().unwrap();
            rt.heap.drop_global(hw);
            rt.heap.drop_global(hs);
        });
    }
}

// ---- routing (P0-D7, V8-A15; plan V3-A13) --------------------------------------------------

/// Runs every internal-method path once on `o` and returns the hooks that ran.
fn hooks_hit(iso: &mut Isolate, o: u64) -> Vec<&'static str> {
    let cx: &mut crate::interp::Cx = &mut iso.cx;
    let rt = cx.rt();
    let key = crate::atoms::intern_str(rt, "x");
    probe::take();
    assert_eq!(property::get(cx, o, key).unwrap(), UNDEFINED);
    assert!(property::get_own_property(cx, o, key).unwrap().is_none());
    assert!(!property::has_own_property(cx, o, key));
    assert!(property::set_with_receiver(cx, o, key, int32_bits(1), o).unwrap());
    let desc = Desc::data(int32_bits(2), attr::DEFAULT);
    assert!(property::define_own_property(cx, o, key, desc).unwrap());
    // CreateDataPropertyOrThrow: a routed kind skips the fast path.
    property::define(cx, o, key, int32_bits(3)).unwrap();
    assert!(property::delete(cx, o, key, true).unwrap());
    let keys = property::own_property_keys(cx, o);
    assert_eq!(object::array_len(keys), 0);
    probe::take()
}

/// Each hook (exotic.rs: get, own_property, own_attrs; property.rs: set, define, delete, own
/// keys) sends an ARGUMENTS cell to arguments.rs and a MODULE_NAMESPACE cell to module.rs;
/// the stubs answer "ordinary", so the ordinary results stay. A plain object reaches none.
#[test]
fn each_hook_sends_the_routed_kinds_to_their_stubs() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let rt: &Runtime = &iso.rt;
        let a = arguments::new_arguments_object(rt, rt.root(Root::ObjectPrototype));
        let ha = rt.heap.new_global(val(a));
        let m = module::new_module_record(rt);
        let names = object::new_list(rt, 0, UNDEFINED);
        let ns = module::new_namespace(rt, m, names);
        let hn = rt.heap.new_global(val(ns));
        let plain = object::new_plain_object(rt);
        let hp = rt.heap.new_global(val(plain));
        assert_eq!(object::proto_of(ns), NULL);
        for (o, kind) in [(a, "arguments"), (ns, "module")] {
            let hits = hooks_hit(&mut iso, o);
            for hook in [
                "get",
                "own_property",
                "own_attrs",
                "set",
                "define_own_property",
                "delete",
                "own_property_keys",
            ] {
                let want = format!("{kind} {hook}");
                assert!(hits.iter().any(|h| *h == want), "{want} not in {hits:?}");
            }
            assert!(hits.iter().all(|h| h.starts_with(kind)), "{hits:?}");
            let defines = format!("{kind} define_own_property");
            let n = hits.iter().filter(|h| **h == defines).count();
            assert_eq!(
                n, 2,
                "define_own_property and define reach the hook: {hits:?}"
            );
        }
        assert_eq!(hooks_hit(&mut iso, plain), Vec::<&str>::new());
        let rt: &Runtime = &iso.rt;
        for h in [ha, hn, hp] {
            rt.heap.drop_global(h);
        }
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

/// A new routed object of `kind` ("arguments": an ARGUMENTS object that inherits from
/// %Object.prototype%; "module": a MODULE_NAMESPACE object with no exports).
fn new_routed(rt: &Runtime, kind: &str) -> u64 {
    if kind == "arguments" {
        return arguments::new_arguments_object(rt, rt.root(Root::ObjectPrototype));
    }
    let m = module::new_module_record(rt);
    let names = object::new_list(rt, 0, UNDEFINED);
    module::new_namespace(rt, m, names)
}

/// True if `hits` holds the hook `want`.
fn hit(hits: &[&str], want: &str) -> bool {
    hits.contains(&want)
}

/// CX-VM1, CX-VM2, VF-FORIN: the integer-index paths (`get_index`, `has_index`,
/// `delete_index`), the `[[Set]]` walk through a routed prototype (§10.1.9.2 step 1.b:
/// parent.[[Set]]) and `for-in` (§14.7.5.9: [[OwnPropertyKeys]]) reach the routed hooks, on
/// the routed object and on a child that inherits from it. A routed object that becomes a
/// prototype sets `shape::proto_index_keys`: its index keys are not in its storage. A plain
/// object and its child reach no hook and leave the flag off.
#[test]
fn index_paths_set_walk_and_for_in_reach_the_routed_hooks() {
    for kind in ["arguments", "module", "plain"] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, true));
            let cx: &mut crate::interp::Cx = &mut iso.cx;
            let rt: &Runtime = &iso.rt;
            assert!(!shape::proto_index_keys(rt), "a fresh isolate");
            let routed = kind != "plain";
            let o = if routed {
                new_routed(rt, kind)
            } else {
                object::new_plain_object(rt)
            };
            let ho = rt.heap.new_global(val(o));
            // The hooks that a path on `o` or its child must hit (none for a plain object).
            let check = |hits: Vec<&'static str>, hook: &str, what: &str| {
                if routed {
                    let want = format!("{kind} {hook}");
                    assert!(hit(&hits, &want), "{what}: {want} not in {hits:?}");
                } else {
                    assert_eq!(hits, Vec::<&str>::new(), "{what}");
                }
            };
            // CX-VM1: the index paths on the object itself.
            probe::take();
            assert_eq!(property::get_index(cx, o, 0).unwrap(), UNDEFINED);
            check(probe::take(), "get", "get_index");
            assert!(!property::has_index(cx, o, 0));
            check(probe::take(), "own_attrs", "has_index");
            assert!(property::delete_index(cx, o, 0, true).unwrap());
            check(probe::take(), "delete", "delete_index");
            // ... and on a child: `o` becomes a prototype.
            let child = object::new_object(rt, o);
            let hc = rt.heap.new_global(val(child));
            assert_eq!(shape::proto_index_keys(rt), routed, "{kind} as a prototype");
            probe::take();
            assert_eq!(property::get_index(cx, child, 0).unwrap(), UNDEFINED);
            check(probe::take(), "get", "get_index on the child");
            assert!(!property::has_index(cx, child, 0));
            check(probe::take(), "own_attrs", "has_index on the child");
            // CX-VM2: the set walk asks the routed prototype's [[Set]]; the stub answers
            // OrdinarySet, so the store lands on the receiver.
            let x = crate::atoms::intern_str(rt, "x");
            assert!(property::set_with_receiver(cx, child, x, int32_bits(1), child).unwrap());
            check(probe::take(), "set", "the set walk");
            assert_eq!(
                props::own_get(child, x),
                Some((int32_bits(1), attr::DEFAULT))
            );
            assert!(props::own_get(o, x).is_none());
            // VF-FORIN: for-in asks [[OwnPropertyKeys]] of the routed object, also as a
            // prototype.
            let keys = property::for_in_keys(cx, o).unwrap();
            assert_eq!(object::array_len(keys), 0);
            check(probe::take(), "own_property_keys", "for-in");
            let keys = property::for_in_keys(cx, child).unwrap();
            assert_eq!(object::array_len(keys), 1, "the child's x");
            assert_eq!(object::array_get(keys, 0), Some(x));
            check(probe::take(), "own_property_keys", "for-in on the child");
            for h in [ho, hc] {
                rt.heap.drop_global(h);
            }
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// Takes the pending exception and checks that it is the module.rs probe's ReferenceError.
fn take_probe_error(cx: &mut crate::interp::Cx, what: &str) {
    let rt = cx.rt();
    let e = rt.take_pending();
    assert!(object::is_object(e), "{what}: an error is pending");
    let name = crate::atoms::intern_str(rt, "name");
    let n = property::get_from(cx, e, name).unwrap();
    let message = crate::atoms::intern_str(cx.rt(), "message");
    let m = property::get_from(cx, e, message).unwrap();
    assert_eq!(
        (
            crate::string::to_rust_string(n),
            crate::string::to_rust_string(m)
        ),
        ("ReferenceError".to_string(), "probe".to_string()),
        "{what}"
    );
}

/// VF-TDZ: the routed value hooks can throw. A namespace `[[Get]]` and `[[GetOwnProperty]]`
/// of an uninitialized binding throw a ReferenceError (§10.4.6.8, §10.4.6.5); `[[Get]]`,
/// `[[GetOwnProperty]]`, the builtin ABI's HasOwnProperty and `for-in` pass the throw on.
/// The module.rs stubs throw while `probe::set_throw(true)` (test builds only).
#[test]
fn routed_value_hooks_can_throw() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let cx: &mut crate::interp::Cx = &mut iso.cx;
            let rt: &Runtime = &iso.rt;
            let ns = new_routed(rt, "module");
            let hn = rt.heap.new_global(val(ns));
            let x = crate::atoms::intern_str(rt, "x");
            // `x` as an ordinary property (the define stub answers "ordinary"), so the
            // ordinary `[[OwnPropertyKeys]]` lists it for for-in.
            let desc = Desc::data(int32_bits(1), attr::DEFAULT);
            assert!(property::define_own_property(cx, ns, x, desc).unwrap());
            probe::set_throw(true);
            assert!(property::get(cx, ns, x).is_err());
            take_probe_error(cx, "[[Get]]");
            assert!(property::get_own_property(cx, ns, x).is_err());
            take_probe_error(cx, "[[GetOwnProperty]]");
            {
                let mut abi = crate::Cx::new(cx);
                let o: crate::Local<'_, crate::JsObject> = crate::Local::from_bits(ns);
                assert!(abi.has_own_property(o, "x").is_err());
            }
            take_probe_error(cx, "HasOwnProperty (builtin ABI)");
            assert!(property::for_in_keys(cx, ns).is_err());
            take_probe_error(cx, "for-in");
            // `get_data_from` runs no JS and cannot throw: it does not ask a routed hook, so
            // it reads the ordinary `x` (X-CODE1).
            probe::take();
            assert_eq!(property::get_data_from(cx, ns, x), Some(int32_bits(1)));
            assert_eq!(rt.pending(), 0, "get_data_from leaves nothing pending");
            assert!(
                probe::take().is_empty(),
                "get_data_from asks no routed hook"
            );
            probe::set_throw(false);
            assert_eq!(property::get(cx, ns, x).unwrap(), int32_bits(1));
            assert!(property::get_own_property(cx, ns, x).unwrap().is_some());
            {
                let mut abi = crate::Cx::new(cx);
                let o: crate::Local<'_, crate::JsObject> = crate::Local::from_bits(ns);
                assert!(abi.has_own_property(o, "x").unwrap());
            }
            rt.heap.drop_global(hn);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// X-CODE1: the engine's diagnostic reads (an error's "Name: message" header, made for every
/// new error) do not ask a routed hook. A namespace on the ReferenceError chain whose `[[Get]]`
/// throws would otherwise make a new ReferenceError whose header asks the hook again: a
/// stack overflow. Only the spec `[[Get]]` asks the hook; the header reads the namespace's
/// ordinary storage.
#[test]
fn diagnostic_reads_skip_routed_hooks() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let cx: &mut crate::interp::Cx = &mut iso.cx;
            let rt: &Runtime = &iso.rt;
            let ns = new_routed(rt, "module");
            let hn = rt.heap.new_global(val(ns));
            let text = crate::atoms::intern_str(rt, "NS");
            let ht = rt.heap.new_global(val(text));
            let name = rt.atom(Atom::Name);
            // `name` as an ordinary property of the namespace (the define stub answers
            // "ordinary").
            let desc = Desc::data(text, attr::DEFAULT);
            assert!(property::define_own_property(cx, ns, name, desc).unwrap());
            // ReferenceError.prototype has no own `name`; its chain reaches the namespace.
            let rep = rt.root(Root::ReferenceErrorPrototype);
            assert!(property::delete(cx, rep, name, true).unwrap());
            assert!(property::set_prototype_of(rt, rep, ns));
            probe::set_throw(true);
            probe::take();
            let x = crate::atoms::intern_str(rt, "x");
            assert!(property::get(cx, ns, x).is_err());
            assert_eq!(
                probe::take(),
                ["module get"],
                "only the spec [[Get]] asks the hook"
            );
            let e = rt.take_pending();
            assert!(object::is_object(e));
            assert_eq!(crate::error::error_header(cx, e), "NS: probe");
            assert_eq!(rt.pending(), 0, "error_header leaves nothing pending");
            let (stack, _) = props::own_get(e, rt.atom(Atom::Stack)).expect("an own stack");
            let stack = crate::string::to_rust_string(stack);
            assert!(stack.starts_with("NS: probe"), "{stack}");
            assert!(probe::take().is_empty(), "the header asks no routed hook");
            probe::set_throw(false);
            assert!(property::set_prototype_of(
                rt,
                rep,
                rt.root(Root::ErrorPrototype)
            ));
            rt.heap.drop_global(ht);
            rt.heap.drop_global(hn);
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// Runs `make` under every dispatch, with and without GC stress, on its own thread; returns
/// each run with the hooks it hit.
fn runs_with_hooks(
    make: impl Fn() -> VerifiedUnit + Copy + Send + 'static,
) -> Vec<(String, Run, Vec<&'static str>)> {
    let mut out = Vec::new();
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            let (r, hits) = on_big_stack(move || {
                probe::take();
                let r = run_in(config(d, stress), make());
                (r, probe::take())
            });
            out.push((format!("{} stress={stress}", d.name()), r, hits));
        }
    }
    out
}

/// V8-A15: `CreateMappedArguments` reaches `arguments::create` with `mapped` true,
/// `CreateUnmappedArguments` with false; both still make today's arguments array.
#[test]
fn arguments_opcodes_reach_arguments_create() {
    fn make() -> VerifiedUnit {
        // f1: function m() { return arguments.length; } (mapped); f2: the same, unmapped.
        let mut m = Asm::new(meta("m", 0, 0, 0, 0));
        let r = m.b.new_register();
        m.b.create_mapped_arguments().star(r);
        m.get_named(r, "length");
        m.b.return_();
        let mut u = Asm::new(meta("u", 0, 0, 0, 0));
        let r = u.b.new_register();
        u.b.create_unmapped_arguments().star(r);
        u.get_named(r, "length");
        u.b.return_();
        let mut s = Asm::script(0);
        let t = s.b.new_register();
        for (i, name) in ["m", "u"].iter().enumerate() {
            let k = s.b.function_constant(FunctionId(i as u32 + 1));
            s.b.create_closure(k, s.closure, 0);
            s.sta_global(name);
        }
        // m(1, 2) + u(1, 2, 3) * 10
        let args = s.a;
        s.b.lda_smi(1)
            .star(args.get(0))
            .lda_smi(2)
            .star(args.get(1))
            .lda_smi(3);
        s.b.star(args.get(2));
        s.lda_global("u");
        s.b.star(s.f).call_undefined_receiver(s.f, args, 3, s.call);
        s.b.mul_smi(10, s.bin).star(t);
        s.lda_global("m");
        s.b.star(s.f)
            .call_undefined_receiver2(s.f, args.get(0), args.get(1), s.call)
            .add(t, s.bin)
            .return_();
        unit("", vec![s.finish(), m.finish(), u.finish()])
    }
    for (label, r, hits) in runs_with_hooks(make) {
        assert_eq!(r.result, Ok("32".to_string()), "{label}");
        assert_eq!(
            hits,
            ["arguments create unmapped", "arguments create mapped"],
            "{label}"
        );
    }
}

/// The cold bodies of `LdaModuleVariable` and `StaModuleVariable` call the `slow::module`
/// stubs, which throw today's "module variables" error.
#[test]
fn module_variable_opcodes_reach_slow_module() {
    fn lda() -> VerifiedUnit {
        let mut s = Asm::script(0);
        s.b.lda_module_variable(0, 0).return_();
        unit("", vec![s.finish()])
    }
    fn sta() -> VerifiedUnit {
        let mut s = Asm::script(0);
        s.b.lda_smi(1).sta_module_variable(0, 0).return_();
        unit("", vec![s.finish()])
    }
    for (make, hook) in [
        (lda as fn() -> VerifiedUnit, "module lda_module_variable"),
        (sta, "module sta_module_variable"),
    ] {
        for (label, r, hits) in runs_with_hooks(make) {
            let e = r.result.expect_err("throws");
            assert_eq!(
                e.to_string(),
                "test.js:1:1: Error: not implemented in ep9 v0: module variables",
                "{label}"
            );
            assert_eq!(hits, [hook], "{label}");
        }
    }
}

/// P0-VM3 step 2 (task 8): `CallRuntime` of ids 36-43, 46 and 47 reaches its stub in
/// `slow/{async_gen, module, eval}.rs`, which throws today's "not implemented" error with the
/// id's name (the text of the old catch-all arm). Ids 32-35 are built (M3-P1 ASYNC,
/// `slow/async_fn.rs`; `tests/async_fn.rs` checks their routing).
#[test]
fn abi5_runtime_ids_reach_their_stubs() {
    const ROUTED: [(RuntimeId, &str); 10] = [
        (RuntimeId::AsyncGeneratorEnter, "async_gen"),
        (RuntimeId::AsyncGeneratorAwait, "async_gen"),
        (RuntimeId::AsyncGeneratorYield, "async_gen"),
        (RuntimeId::AsyncGeneratorResolve, "async_gen"),
        (RuntimeId::AsyncGeneratorReject, "async_gen"),
        (RuntimeId::CreateAsyncFromSyncIterator, "async_gen"),
        (RuntimeId::GetImportMeta, "module"),
        (RuntimeId::DynamicImport, "module"),
        (RuntimeId::DeclareEvalAnnexBFunction, "eval"),
        (RuntimeId::StoreEvalAnnexBFunction, "eval"),
    ];
    let ids: Vec<u16> = ROUTED.iter().map(|&(id, _)| id as u16).collect();
    assert_eq!(ids, [36, 37, 38, 39, 40, 41, 42, 43, 46, 47]);
    for (id, file) in ROUTED {
        let info = id.info();
        let argc = info.arity.expect("fixed arity");
        let make = move || {
            let mut s = Asm::script(0);
            let args = if argc == 0 {
                RegisterList::EMPTY
            } else {
                s.b.new_register_list(argc)
            };
            for i in 0..argc {
                s.b.lda_smi(i as i32 + 1).star(args.get(i));
            }
            s.b.call_runtime(id, args, argc).return_();
            unit("", vec![s.finish()])
        };
        let hook = format!("{file} {}", info.name);
        for (label, r, hits) in runs_with_hooks(make) {
            let e = r.result.expect_err("throws");
            assert_eq!(
                e.to_string(),
                format!(
                    "test.js:1:1: Error: not implemented in ep9 v0: {}",
                    info.name
                ),
                "{} {label}",
                info.name
            );
            assert_eq!(hits, [hook.as_str()], "{} {label}", info.name);
        }
    }
}
