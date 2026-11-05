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
use crate::frame::{UNDEFINED, int32_bits};
use crate::object::{self, argsf, attr, cell, fun, modf, nsf, obj, realmf};
use crate::ordered_table::{self, MAP_WIDTH, SET_WIDTH, otf};
use crate::runtime::{Root, Runtime, val};
use crate::{Dispatch, Isolate, arguments, kinds, module, property, props, shape, versions};

// ---- versions, kinds, fixed fields ------------------------------------------------------------

#[test]
fn layout_3_versions_kinds_and_fixed_fields() {
    // OBJECT_LAYOUT 4 (M4-P0) and 5 (M5-P0) keep every layout-3 kind and field checked here;
    // the version numbers are pinned by `tests::layout5`.
    assert!(black_box(versions::OBJECT_LAYOUT_VERSION) >= 3);
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
    // MODULE: 16 Values after word0; REALM: 5 (7 since OBJECT_LAYOUT 5, `tests::layout5`).
    assert_eq!(modf::SIZE, 8 + 8 * 16);
    assert_eq!((modf::STATUS, modf::HOST_DEFINED), (8, 128));
    assert_eq!(realmf::SIZE, 8 + 8 * 7);
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
/// `ordered_table::register_kinds` call): a second registration of each is refused. The
/// never-assigned id [`NEVER_ASSIGNED`] is free (C-24: a probe never uses "the next free id",
/// which the next appended kind takes; until M5-P3 this probe used 29).
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
    let free = KindId::new(NEVER_ASSIGNED).unwrap();
    assert_eq!(iso.rt.heap.register_kind(free, k("free")), Ok(()));
}

// ---- C-24: the kind ids and the IC handler kinds are append-only lists (M5-P3 M1A-CODE) -----
//
// MODES-CHECK-w17 O4: each append-only list states its field width, a full list is refused
// (never wrapped), and a test fills it. Negative tests probe a value that is never assigned.

/// The kind id that is never assigned (the `kinds.rs` module doc, C-24): no range holds it.
const NEVER_ASSIGNED: u8 = 255;

/// Every kind id the table declares (`kinds.rs`), by range.
fn declared_kinds() -> Vec<(&'static str, KindId)> {
    vec![
        ("SEQ_LATIN1", kinds::SEQ_LATIN1),
        ("SEQ_UTF16", kinds::SEQ_UTF16),
        ("CONS", kinds::CONS),
        ("SLICED", kinds::SLICED),
        ("THIN", kinds::THIN),
        ("EXT_LATIN1", kinds::EXT_LATIN1),
        ("EXT_UTF16", kinds::EXT_UTF16),
        ("SYMBOL", kinds::SYMBOL),
        // OBJECT_LAYOUT 7 (spec-temporal B2): its own single-id range.
        ("BIGINT", kinds::BIGINT),
        ("OBJECT", kinds::OBJECT),
        ("FUNCTION", kinds::FUNCTION),
        ("ARRAY", kinds::ARRAY),
        ("ERROR", kinds::ERROR),
        ("ITERATOR", kinds::ITERATOR),
        ("PRIMITIVE_WRAPPER", kinds::PRIMITIVE_WRAPPER),
        ("BOUND_FUNCTION", kinds::BOUND_FUNCTION),
        ("GENERATOR", kinds::GENERATOR),
        ("SLOT_OBJECT", kinds::SLOT_OBJECT),
        ("ARGUMENTS", kinds::ARGUMENTS),
        ("MODULE_NAMESPACE", kinds::MODULE_NAMESPACE),
        ("PROXY", kinds::PROXY),
        ("PROXY_CALLABLE", kinds::PROXY_CALLABLE),
        ("PROPS", kinds::PROPS),
        ("ELEMENTS", kinds::ELEMENTS),
        ("CONTEXT", kinds::CONTEXT),
        ("ACCESSOR_PAIR", kinds::ACCESSOR_PAIR),
        ("FEEDBACK", kinds::FEEDBACK),
        ("POLY_IC", kinds::POLY_IC),
        ("IC_HANDLER", kinds::IC_HANDLER),
        ("SLOTS", kinds::SLOTS),
        ("DESCRIPTORS", kinds::DESCRIPTORS),
        ("DESC_INDEX", kinds::DESC_INDEX),
        ("TRANSITIONS", kinds::TRANSITIONS),
        ("PROTO_INFO", kinds::PROTO_INFO),
        ("MODULE", kinds::MODULE),
        ("ORDERED_TABLE", kinds::ORDERED_TABLE),
        ("REALM", kinds::REALM),
        ("VALIDITY_CELL", kinds::VALIDITY_CELL),
        ("WEAK_CELL", kinds::WEAK_CELL),
        ("ELEMENTS_DOUBLE", kinds::ELEMENTS_DOUBLE),
        ("ELEMENTS_DICT", kinds::ELEMENTS_DICT),
        ("FUSE", kinds::FUSE),
        ("SHAPE", kinds::SHAPE),
    ]
}

/// The kind id list (H17, C-24): a u8 in word0 byte 5, 0 refused, so the list is full at 255
/// ids and has no 256th (no wrap: `KindId` is a non-zero u8). The ids as built at M1a are
/// pinned (an append adds a row here; a renumbering fails), each lies in its range of the
/// `kinds.rs` table, and the routed range 25..=28 is closed: exactly its 4 kinds, so a new
/// routed kind needs its own compare.
#[test]
fn kind_ids_are_an_append_only_byte_list() {
    assert_eq!(KindId::new(0), None);
    assert_eq!(KindId::new(u8::MAX).map(KindId::get), Some(255));
    let all = declared_kinds();
    let ids: Vec<u8> = all.iter().map(|(_, k)| k.get()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "a kind id is used twice: {ids:?}");
    assert_eq!(
        ids,
        [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 64, 65,
            66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 128
        ],
        "kind ids as built (append-only: a new id is added at the end of its range)"
    );
    for (name, k) in &all {
        let id = k.get();
        let range = match id {
            1..=7 => "strings",
            8 => "symbol",
            9 => "bigint",
            16..=63 => "JS objects",
            64..=127 => "internal cells",
            128 => "shape",
            _ => panic!("{name} = {id} is in no range of the kinds.rs table"),
        };
        assert!(!range.is_empty());
    }
    // The never-assigned probe id is no declared kind and in no range.
    assert!(!ids.contains(&NEVER_ASSIGNED));
    const { assert!(NEVER_ASSIGNED > 128) };
    // `is_routed` is the closed range 25..=28: exactly the 4 routed kinds, of all 256 bytes.
    let routed: Vec<u8> = (0..=u8::MAX)
        .filter(|&k| crate::exotic::is_routed(k))
        .collect();
    assert_eq!(
        routed,
        [
            kinds::ARGUMENTS.get(),
            kinds::MODULE_NAMESPACE.get(),
            kinds::PROXY.get(),
            kinds::PROXY_CALLABLE.get()
        ]
    );
}

/// A kind test on Value bits (`kinds::is_*`).
type KindTest = fn(u64) -> bool;

/// A cell of the never-assigned kind is none of the kinds: every kind test on Value bits
/// says no (a test that probed "the next free id" would turn true when that id is appended).
#[test]
fn a_cell_of_a_never_assigned_kind_passes_no_kind_test() {
    let iso = isolate(config(Dispatch::DEFAULT, false));
    fn dummy(_: ep9_heap::CellPtr, _: &mut ep9_heap::Tracer<'_>) {}
    let probe = KindId::new(NEVER_ASSIGNED).unwrap();
    let kind = CellKind {
        name: "never assigned",
        size: CellSize::Fixed(16),
        trace: dummy,
    };
    assert_eq!(iso.rt.heap.register_kind(probe, kind), Ok(()));
    let c = iso.rt.heap.alloc(probe, 16).expect("a 16 B cell");
    // No GC point until the last read: `c` is only read here.
    let b = c.to_value().to_bits();
    assert_eq!(kinds::cell_kind(b), NEVER_ASSIGNED);
    let tests: [(&str, KindTest); 11] = [
        ("is_string", kinds::is_string),
        ("is_symbol", kinds::is_symbol),
        ("is_object", kinds::is_object),
        ("is_function", kinds::is_function),
        ("is_callable", kinds::is_callable),
        ("is_array", kinds::is_array),
        ("is_error", kinds::is_error),
        ("is_iterator", kinds::is_iterator),
        ("is_context", kinds::is_context),
        ("is_wrapper", kinds::is_wrapper),
        ("is_bound", kinds::is_bound),
    ];
    for (name, t) in tests {
        assert!(!t(b), "{name} of a kind-{NEVER_ASSIGNED} cell");
    }
    assert!(!crate::exotic::is_routed(NEVER_ASSIGNED));
    black_box(b);
}

/// The smi handler kinds (H16, C-24; MODES-CHECK-w20 MC-w20-10): a 4-bit field (bits 0-3)
/// below the byte offset (bits 4-31). Kinds 1-6 and 9-14 are assigned; 0 and 15 are not, and
/// the hit and widening helpers leave them alone. The field is full for the paired kinds:
/// the next in-object / out-of-line pair (15, 16) does not fit, because 16 would wrap into
/// the offset. So a new smi handler kind past the field is an IC_HANDLER cell kind (one more
/// load), never a wrapped value. The IC_HANDLER kinds (the int32 at +8) are 1-8.
#[test]
fn ic_handler_kind_lists_state_their_width() {
    use crate::ic::{hk, smi};
    assert_eq!(smi::KIND_MASK, (1 << smi::SHIFT) - 1);
    assert_eq!(smi::KIND_MASK, 0xF, "a 4-bit field");
    let assigned = [
        smi::FIELD_IN_OBJECT,
        smi::FIELD_OUT_OF_LINE,
        smi::ARRAY_LENGTH,
        smi::STRING_LENGTH,
        smi::SLOW,
        smi::MEGA,
        smi::FIELD_INT32_IN_OBJECT,
        smi::FIELD_INT32_OUT_OF_LINE,
        smi::FIELD_NUMBER_IN_OBJECT,
        smi::FIELD_NUMBER_OUT_OF_LINE,
        smi::FIELD_HEAP_IN_OBJECT,
        smi::FIELD_HEAP_OUT_OF_LINE,
    ];
    assert_eq!(assigned, [1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14]);
    for k in assigned {
        assert_eq!(k & !smi::KIND_MASK, 0, "kind {k} fits the field");
    }
    // 0 and 15 (the probe at the top of the range): no helper treats them as a checked field
    // kind, and the field handler made at an offset keeps them (no wrap into the offset). 7
    // and 8 are free for the next checked representation (H16 K9), so they are not probed
    // (README §3.4: a negative test probes the top of the range, never the next free value).
    for k in [0, 15] {
        assert!(!assigned.contains(&k));
        assert!(!smi::is_checked(k), "kind {k}");
        let x = (40 << smi::SHIFT) | k;
        assert_eq!(smi::plain(x), x, "kind {k}");
        assert_eq!(x >> smi::SHIFT, 40);
    }
    // Every checked kind made from a field handler stays in the field, at every offset up
    // to the largest one the 28 offset bits hold.
    let max_off = i32::MAX >> smi::SHIFT;
    for off in [0, 8, 1 << 20, max_off] {
        for base in [smi::FIELD_IN_OBJECT, smi::FIELD_OUT_OF_LINE] {
            for r in [shape::rep::INT32, shape::rep::DOUBLE, shape::rep::HEAP] {
                let x = smi::checked((off << smi::SHIFT) | base, r).expect("a checked kind");
                assert!(smi::is_checked(x & smi::KIND_MASK));
                assert_eq!(x >> smi::SHIFT, off, "offset {off}");
                assert_eq!(smi::plain(x), (off << smi::SHIFT) | base);
            }
        }
    }
    // Full: the next checked pair would be (15, 16); 16 does not fit, it would wrap to 0 and
    // move the offset by one.
    let next_pair = (
        smi::FIELD_HEAP_OUT_OF_LINE + 1,
        smi::FIELD_HEAP_OUT_OF_LINE + 2,
    );
    assert_eq!(next_pair, (15, 16));
    assert!(next_pair.1 > smi::KIND_MASK);
    assert_eq!(((40 << smi::SHIFT) + next_pair.1) >> smi::SHIFT, 41);
    // The IC_HANDLER cell kinds: an int32 field, 1-8 as built, 0 never assigned.
    let cell_kinds = [
        hk::PROTO_LOAD,
        hk::MISSING,
        hk::ADD,
        hk::GLOBAL,
        hk::CONSTRUCT_SITE,
        hk::INSTANCEOF,
        hk::CLONE,
        hk::ALLOC_SITE,
    ];
    assert_eq!(cell_kinds, [1, 2, 3, 4, 5, 6, 7, 8]);
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
            iso.with_cx_checkpoint(|cx| cx.run_script(two_functions(), "realm.js").map(|_| ()))
                .unwrap();
            take_output();
            let rt: &Runtime = &iso.rt;
            let realm = rt.root(Root::Realm);
            assert_eq!(kinds::cell_kind(realm), kinds::REALM.get());
            assert_eq!(iso.cx.realm, realm, "Cx::realm is a copy of the REALM root");
            let f = iso.global_raw("f").unwrap().to_bits();
            let g = iso.global_raw("g").unwrap().to_bits();
            assert!(object::function_info(g).is_generator());
            assert_realm(rt, f, "a bytecode function");
            assert_realm(rt, g, "a generator function");
            assert_realm(
                rt,
                iso.global_raw("print").unwrap().to_bits(),
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
            // Append-safe (README §3.4): a pin on one id, not on the count of the list.
            assert_eq!(IntrinsicId::RealmFuses as usize, 72);
            for id in IntrinsicId::ALL {
                assert_eq!(
                    object::list_get(list, id as usize),
                    intrinsic(rt, id),
                    "{}",
                    id.name()
                );
            }
            // Ids 45-63 are declared (P0-D3); the VM table makes 45-49 (M3-P2 ASYNCGEN) and 63
            // (%ThrowTypeError%, M3-P2 VM-ARGS step 2); the others are not made without an
            // embedder table.
            for id in IntrinsicId::ALL[45..50]
                .iter()
                .chain(&IntrinsicId::ALL[63..64])
            {
                assert!(crate::kinds::is_object(intrinsic(rt, *id)), "{}", id.name());
            }
            // Ids 64-71 (M4-P0) are not made yet either (`tests::layout4`).
            for id in IntrinsicId::ALL[50..63]
                .iter()
                .chain(&IntrinsicId::ALL[64..72])
            {
                assert_eq!(intrinsic(rt, *id), UNDEFINED, "{}", id.name());
            }
            // Id 72 (M6-P0) is the realm's static fuse block, a FUSE cell, not an object
            // (made at realm init since M6-P2 FUSE-PI; `tests::fuse`).
            let fuses = intrinsic(rt, IntrinsicId::RealmFuses);
            assert_eq!(crate::kinds::cell_kind(fuses), crate::kinds::FUSE.get());
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
    // REALM: 7 Values (5 before OBJECT_LAYOUT 5).
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
/// keys) on an ARGUMENTS cell with no elements gives the ordinary results. The arguments.rs
/// hooks are built (M3-P2 VM-ARGS; `tests/arguments.rs`) and note nothing. A plain object
/// reaches no hook either. (A MODULE_NAMESPACE cell reaches module.rs, which gives the
/// §10.4.6 answers since M3-P2 MODULE: `tests::module`.)
#[test]
fn each_hook_sends_the_routed_kinds_to_their_stubs() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let rt: &Runtime = &iso.rt;
        let a = arguments::new_arguments_object(rt, rt.root(Root::ObjectPrototype));
        let ha = rt.heap.new_global(val(a));
        let plain = object::new_plain_object(rt);
        let hp = rt.heap.new_global(val(plain));
        assert_eq!(hooks_hit(&mut iso, a), Vec::<&str>::new());
        assert_eq!(hooks_hit(&mut iso, plain), Vec::<&str>::new());
        let rt: &Runtime = &iso.rt;
        for h in [ha, hp] {
            rt.heap.drop_global(h);
        }
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

/// CX-VM1, CX-VM2, VF-FORIN on a plain object: the integer-index paths (`get_index`,
/// `has_index`, `delete_index`), the `[[Set]]` walk and `for-in` reach no routed hook, on
/// the object and on a child that inherits from it, and the object as a prototype leaves
/// `shape::proto_index_keys` off. (Arguments objects with elements:
/// `index_paths_set_walk_and_for_in_reach_the_routed_hooks`; a namespace: `tests::module`
/// `index_paths_set_walk_and_for_in_reach_the_namespace`.)
#[test]
fn index_paths_set_walk_and_for_in_of_a_plain_object_reach_no_hook() {
    on_big_stack(move || {
        let mut iso = isolate(config(Dispatch::DEFAULT, true));
        let cx: &mut crate::interp::Cx = &mut iso.cx;
        let rt: &Runtime = &iso.rt;
        assert!(!shape::proto_index_keys(rt), "a fresh isolate");
        let o = object::new_plain_object(rt);
        let ho = rt.heap.new_global(val(o));
        probe::take();
        assert_eq!(property::get_index(cx, o, 0).unwrap(), UNDEFINED);
        assert!(!property::has_index(cx, o, 0));
        assert!(property::delete_index(cx, o, 0, true).unwrap());
        let child = object::new_object(rt, o);
        let hc = rt.heap.new_global(val(child));
        assert!(
            !shape::proto_index_keys(rt),
            "a plain object as a prototype"
        );
        assert_eq!(property::get_index(cx, child, 0).unwrap(), UNDEFINED);
        assert!(!property::has_index(cx, child, 0));
        let x = crate::atoms::intern_str(rt, "x");
        assert!(property::set_with_receiver(cx, child, x, int32_bits(1), child).unwrap());
        assert_eq!(
            props::own_get(child, x),
            Some((int32_bits(1), attr::DEFAULT))
        );
        assert!(props::own_get(o, x).is_none());
        let keys = property::for_in_keys(cx, o).unwrap();
        assert_eq!(object::array_len(keys), 0);
        let keys = property::for_in_keys(cx, child).unwrap();
        assert_eq!(object::array_len(keys), 1, "the child's x");
        assert_eq!(object::array_get(keys, 0), Some(x));
        assert_eq!(probe::take(), Vec::<&str>::new());
        for h in [ho, hc] {
            rt.heap.drop_global(h);
        }
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

/// `for-in` of `o` (§14.7.5.9) as text.
fn for_in_text(cx: &mut crate::interp::Cx, o: u64) -> Vec<String> {
    let keys = property::for_in_keys(cx, o).unwrap();
    (0..object::array_len(keys))
        .map(|j| crate::string::to_rust_string(object::array_get(keys, j).unwrap()))
        .collect()
}

/// CX-VM1, CX-VM2, VF-FORIN (Codex round 2, R3-3) on arguments objects WITH elements:
/// `f(1, 2, 3)` of a sloppy `function f(a, b)` (indices 0 and 1 mapped) and of a strict
/// `function g(a)` (unmapped). The elements live in the ELEMENTS list that only the
/// arguments.rs hooks read, not in the ordinary storage, so a path that does not route
/// ARGUMENTS to its hooks gives other results: `own_index` (under `get_index`, `has_index`,
/// `delete_index`) would answer "absent" for every index, `for-in` would list no index, and
/// a child would not see the inherited indices once the object as a prototype did not set
/// `shape::proto_index_keys`. Each of these breaks fails this test, and so does a hook
/// (get, own_property, own_attrs, delete, own keys) that skips ARGUMENTS. The `[[Set]]`
/// walk's routing (§10.1.9.2 step 1.b) is the one path that no result here shows: with a
/// receiver that is not the arguments object, its `[[Set]]` is OrdinarySet (§10.4.4.4), which
/// the ordinary walk does through the routed own_attrs hook. The set checks below guard the
/// result; `tests::module` covers that routing for a namespace. The expected results are
/// node v24's for the same script.
#[test]
fn index_paths_set_walk_and_for_in_reach_the_routed_hooks() {
    use super::arguments::{keys, mapped_a, param, set_param, unmapped_a};
    for (name, make) in [
        ("mapped", mapped_a as fn() -> VerifiedUnit),
        ("unmapped", unmapped_a),
    ] {
        let mapped = name == "mapped";
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, true));
            iso.with_cx_checkpoint(|cx| cx.run_script(make(), "args.js").map(|_| ()))
                .unwrap();
            let a = iso.global_raw("A").expect("A").to_bits();
            let cx: &mut crate::interp::Cx = &mut iso.cx;
            let rt: &Runtime = &iso.rt;
            let ha = rt.heap.new_global(val(a));
            assert_eq!(kinds::cell_kind(a), kinds::ARGUMENTS.get(), "{name}");
            let int = |n: i32| int32_bits(n);
            // CX-VM1: index reads and presence on the object itself (`own_index`).
            for (i, v) in [(0, int(1)), (1, int(2)), (2, int(3)), (3, UNDEFINED)] {
                assert_eq!(property::get_index(cx, a, i).unwrap(), v, "{name} A[{i}]");
                assert_eq!(property::has_index(cx, a, i), i < 3, "{name} {i} in A");
            }
            // A mapped index reads the parameter (§10.4.4.3).
            let v0 = if mapped {
                set_param(cx, a, 0, int(7));
                int(7)
            } else {
                int(1)
            };
            assert_eq!(property::get_index(cx, a, 0).unwrap(), v0, "{name} A[0]");
            // VF-FORIN: the enumerable own keys are the indices.
            assert_eq!(for_in_text(cx, a), ["0", "1", "2"], "{name}");
            assert_eq!(
                keys(cx, a),
                ["0", "1", "2", "length", "callee", "@@iterator"],
                "{name}"
            );
            // A child inherits the indices: the object as a prototype sets the flag, and the
            // index paths walk to it.
            let rt: &Runtime = cx.rt();
            assert!(!shape::proto_index_keys(rt), "{name}: before the child");
            let child = object::new_object(rt, a);
            let hc = rt.heap.new_global(val(child));
            assert!(
                shape::proto_index_keys(rt),
                "{name}: arguments as a prototype"
            );
            assert_eq!(property::get_index(cx, child, 0).unwrap(), v0, "{name}");
            assert_eq!(property::get_index(cx, child, 2).unwrap(), int(3), "{name}");
            assert!(property::has_index(cx, child, 2), "{name}");
            assert!(!property::has_index(cx, child, 3), "{name}");
            assert_eq!(for_in_text(cx, child), ["0", "1", "2"], "{name}");
            // CX-VM2: a set of an inherited writable index lands on the child; the arguments
            // object (and a mapped parameter) keep their value (§10.4.4.4 step 1: the
            // receiver is not the arguments object, so OrdinarySet).
            let k0 = crate::key::index_key(cx.rt(), 0);
            assert!(property::set_with_receiver(cx, child, k0, int(50), child).unwrap());
            assert_eq!(props::own_get(child, k0), Some((int(50), attr::DEFAULT)));
            assert_eq!(property::get_index(cx, a, 0).unwrap(), v0, "{name}");
            if mapped {
                assert_eq!(param(a, 0), int(7), "{name}");
            }
            // A read-only inherited index stops the set (§10.1.9.2 step 2.a).
            let k1 = crate::key::index_key(cx.rt(), 1);
            let ro = Desc {
                writable: Some(false),
                ..Desc::default()
            };
            assert!(property::define_own_property(cx, a, k1, ro).unwrap());
            assert!(!property::set_with_receiver(cx, child, k1, int(60), child).unwrap());
            assert!(props::own_get(child, k1).is_none(), "{name}");
            // Delete constraints: a non-configurable index is not deleted (false, or a
            // TypeError in strict code); a configurable one is, and the child no longer
            // inherits it.
            let k2 = crate::key::index_key(cx.rt(), 2);
            let nc = Desc {
                configurable: Some(false),
                ..Desc::default()
            };
            assert!(property::define_own_property(cx, a, k2, nc).unwrap());
            assert!(!property::delete_index(cx, a, 2, false).unwrap(), "{name}");
            assert!(property::delete_index(cx, a, 2, true).is_err(), "{name}");
            let e = cx.rt().take_pending();
            let n = crate::atoms::intern_str(cx.rt(), "name");
            let n = property::get_from(cx, e, n).unwrap();
            assert_eq!(crate::string::to_rust_string(n), "TypeError", "{name}");
            assert!(property::has_index(cx, a, 2), "{name}");
            assert!(property::delete_index(cx, a, 1, true).unwrap(), "{name}");
            assert!(!property::has_index(cx, a, 1), "{name}");
            assert!(!property::has_own_property(cx, a, k1), "{name}");
            assert_eq!(property::get_index(cx, a, 1).unwrap(), UNDEFINED, "{name}");
            assert_eq!(property::get_index(cx, child, 1).unwrap(), UNDEFINED);
            assert_eq!(for_in_text(cx, a), ["0", "2"], "{name}");
            assert_eq!(for_in_text(cx, child), ["0", "2"], "{name}");
            let rt: &Runtime = &iso.rt;
            for h in [ha, hc] {
                rt.heap.drop_global(h);
            }
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

// VF-TDZ and X-CODE1 (a namespace binding in its TDZ throws through every path; an error
// header asks no namespace hook) are tests of real namespaces since M3-P2 MODULE:
// `tests::module` `a_binding_in_its_tdz_throws_through_every_path` and
// `error_headers_skip_namespace_hooks`.

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
/// `CreateUnmappedArguments` with false; both make an arguments object (M3-P2 VM-ARGS,
/// `tests/arguments.rs`), and no routed hook notes a call.
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
        assert_eq!(hits, Vec::<&str>::new(), "{label}");
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

/// P0-VM3 step 2 (task 8): `CallRuntime` of id 43 (`DynamicImport`) reaches its stub in
/// `slow/module.rs`, which throws today's "not implemented" error with the id's name (the
/// text of the old catch-all arm). The other ABI 5 ids are built: 32-35 (M3-P1 ASYNC,
/// `slow/async_fn.rs`; `tests/async_fn.rs`), 36-41 (M3-P2 ASYNCGEN, `slow/async_gen.rs`;
/// `tests/async_gen.rs`), 42 `GetImportMeta` (M3-P2 MODULE;
/// `tests::module::script_code_has_no_module`) and 46-47 (M3-P2 VM-ARGS, `slow/eval.rs`).
#[test]
fn abi5_runtime_ids_reach_their_stubs() {
    const ROUTED: [(RuntimeId, &str); 1] = [(RuntimeId::DynamicImport, "module")];
    let ids: Vec<u16> = ROUTED.iter().map(|&(id, _)| id as u16).collect();
    assert_eq!(ids, [43]);
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
