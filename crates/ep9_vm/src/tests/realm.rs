//! Realms (REALM, kind 78; `Cx.realm`; H22-Q3) (owner M3-P1 REALMS): the realm switch writes
//! every intrinsic of the callee realm into the current-realm roots and puts the caller's
//! back; the REALM record stays a copy of the roots; a host function runs in its own realm
//! (H20 R5); GetFunctionRealm; what is per realm and what is per isolate. The JS-level tests
//! are in `crates/ep9_builtins/tests/realm.rs`, the embedder ones in `tests/isolate_api.rs`.

use super::{config, isolate, on_big_stack};
use crate::builtins::{self, BuiltinId, IntrinsicId, intrinsic};
use crate::frame::UNDEFINED;
use crate::object::{self, realmf};
use crate::runtime::{Root, Runtime};
use crate::symbol::{self, WellKnownSymbol};
use crate::{Args, Cx, Dispatch, JsResult, Local, natives, realm};

/// Every intrinsic of the current realm, in `IntrinsicId` order.
fn intrinsics(rt: &Runtime) -> Vec<u64> {
    IntrinsicId::ALL
        .iter()
        .map(|&id| intrinsic(rt, id))
        .collect()
}

/// Every entry of the intrinsics list of the REALM record `r`.
fn record_list(r: u64) -> Vec<u64> {
    let list = object::record_get(r, realmf::INTRINSICS);
    (0..object::list_len(list))
        .map(|i| object::list_get(list, i))
        .collect()
}

/// The first realm and a second one: every intrinsic of the second is its own (the core ones
/// and every one a table made differ from the first realm's, the ones nothing made stay
/// undefined), and the records mirror the roots. Inside `with_realm` the roots, the builtin
/// state slots, `%Symbol.prototype%`, `Cx::realm` and `Cx::global_this` are the second
/// realm's; after it, the first realm's again. The well-known symbols and the realm key are
/// one per isolate.
#[test]
fn a_switch_loads_every_intrinsic_and_puts_them_back() {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            let first = iso.cx.realm;
            let first_values = intrinsics(&iso.rt);
            assert_eq!(
                record_list(first),
                first_values,
                "the record mirrors the roots"
            );
            let iterator = symbol::well_known(&iso.rt, WellKnownSymbol::Iterator);
            let key = iso.rt.root(Root::RealmKey);
            let r = iso.with_cx(|cx| {
                let g = cx.create_realm()?;
                let rt: &Runtime = cx.inner().rt();
                // `create_realm` leaves the first realm current.
                assert_eq!(cx.inner().realm, first);
                assert_eq!(intrinsics(rt), first_values);
                let second = realm::of_global(rt, g.bits()).expect("a realm's global");
                assert_ne!(second, first);
                let second_values = record_list(second);
                for (i, id) in IntrinsicId::ALL.iter().enumerate() {
                    let (a, b) = (first_values[i], second_values[i]);
                    if a == UNDEFINED {
                        assert_eq!(b, UNDEFINED, "{} is made in one realm only", id.name());
                    } else if *id == IntrinsicId::RealmFuses {
                        // The realm's static fuse block (M6-P2 FUSE-PI): a FUSE cell, not an
                        // object, and each realm's own.
                        for v in [a, b] {
                            assert_eq!(crate::kinds::cell_kind(v), crate::kinds::FUSE.get());
                        }
                        assert_ne!(a, b, "{} is per realm", id.name());
                    } else {
                        assert!(object::is_object(b), "{}", id.name());
                        assert_ne!(a, b, "{} is per realm", id.name());
                    }
                }
                cx.with_realm(g, |cx| {
                    let inner = cx.inner();
                    let rt: &Runtime = inner.rt();
                    assert_eq!(inner.realm, second);
                    assert_eq!(rt.root(Root::Realm), second);
                    assert_eq!(inner.global_this, g.bits());
                    assert_eq!(rt.global(), g.bits());
                    assert_eq!(intrinsics(rt), second_values);
                    assert_eq!(
                        object::record_get(second, realmf::GLOBAL_LEXICAL),
                        rt.root(Root::Lexical)
                    );
                    // Per isolate: the well-known symbols and the realm key.
                    assert_eq!(symbol::well_known(rt, WellKnownSymbol::Iterator), iterator);
                    assert_eq!(rt.root(Root::RealmKey), key);
                    rt.heap.collect();
                    Ok(())
                })?;
                assert_eq!(cx.inner().realm, first);
                assert_eq!(cx.inner().global_this, rt.global());
                assert_eq!(intrinsics(rt), first_values);
                Ok(())
            });
            assert_eq!(r, Ok(()));
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// `realm()`: the global object of the realm the host function runs in.
fn realm_global<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(cx.global().into())
}

/// A host function made in a second realm runs there when the first realm calls it (H20 R5):
/// its `Cx::global` and the objects it makes are the second realm's, and the first realm is
/// current again after the call. `builtins::existing_function` knows the first realm's
/// builtin functions only: a second realm's `%Array.prototype.values%` never passes the
/// identity test, in either realm. GetFunctionRealm (and `Cx::function_realm_intrinsic`)
/// reads a bound function's target.
#[test]
fn a_host_function_runs_in_its_realm() {
    on_big_stack(|| {
        let mut iso = isolate(config(Dispatch::DEFAULT, false));
        let first = iso.cx.realm;
        let r = iso.with_cx(|cx| {
            let g = cx.create_realm()?;
            let f = cx.with_realm(g, |cx| {
                Ok(cx.new_builtin_closure(realm_global, "realm", 0, Local::undefined()))
            })?;
            let seen = cx.call(f, Local::undefined(), &[])?;
            assert_eq!(seen.bits(), g.bits(), "the closure ran in its realm");
            assert_eq!(cx.inner().realm, first);
            let rt: &Runtime = cx.inner().rt();
            let second = realm::of_global(rt, g.bits()).unwrap();
            assert_eq!(object::function_realm(f.bits()), second);
            assert_eq!(realm::get_function_realm(rt, f.bits()), second);
            let b = crate::bound::new_bound_function(cx.inner(), f.bits(), UNDEFINED, &[])?;
            assert_eq!(realm::get_function_realm(rt, b), second);
            assert_eq!(
                realm::get_function_realm(rt, g.bits()),
                first,
                "not a function"
            );
            let values = BuiltinId::new(natives::VM_BLOCK, natives::vm::ARRAY_VALUES);
            let mine = builtins::existing_function(rt, values).expect("the first realm's");
            assert_eq!(object::function_realm(mine), first);
            cx.with_realm(g, |cx| {
                let rt: &Runtime = cx.inner().rt();
                assert_eq!(builtins::existing_function(rt, values), None);
                Ok(())
            })?;
            assert_eq!(
                realm::intrinsic_in(rt, second, IntrinsicId::ObjectPrototype),
                record_list(second)[IntrinsicId::ObjectPrototype as usize]
            );
            // `Cx::function_realm_intrinsic`: through a bound function too; the current realm
            // for a value that is not a function.
            let array2 = record_list(second)[IntrinsicId::Array as usize];
            let bl: Local<'_> = Local::from_bits(b);
            assert_eq!(
                cx.function_realm_intrinsic(f, IntrinsicId::Array).bits(),
                array2
            );
            assert_eq!(
                cx.function_realm_intrinsic(bl, IntrinsicId::Array).bits(),
                array2
            );
            assert_eq!(
                cx.function_realm_intrinsic(g, IntrinsicId::Array).bits(),
                intrinsic(rt, IntrinsicId::Array)
            );
            Ok(())
        });
        assert_eq!(r, Ok(()));
        iso.collect_garbage();
        iso.verify_heap().unwrap();
    });
}

// ---- lazy namespace groups (INTL-I1-D8; M7-P1 LAZYNS) -----------------------------------------

mod lazy {
    use super::realm_global;
    use crate::builtins::{
        BuiltinDef, BuiltinModule, BuiltinTable, ConstValue, IntrinsicDef, IntrinsicId,
        IntrinsicValue, ObjectProps, PropAttrs, PropDef, PropKey, WellKnownSymbol,
    };

    const OBJECT: IntrinsicValue = IntrinsicValue::Object {
        proto: Some(IntrinsicId::ObjectPrototype),
    };

    /// Eager: `%Reflect%` (a module may fill the slot) on the global object as `Eager`.
    const EAGER: BuiltinModule = BuiltinModule {
        name: "eager",
        block: 101,
        defs: &[BuiltinDef::method("e", 0, realm_global)],
        intrinsics: &[IntrinsicDef {
            id: IntrinsicId::Reflect,
            value: OBJECT,
        }],
        props: &[
            ObjectProps {
                target: IntrinsicId::Reflect,
                props: &[PropDef::method(PropKey::Str("e"), 0)],
            },
            ObjectProps {
                target: IntrinsicId::GlobalObject,
                props: &[PropDef::intrinsic(
                    PropKey::Str("Eager"),
                    IntrinsicId::Reflect,
                    PropAttrs::METHOD,
                )],
            },
        ],
    };

    /// Group 0, key `Temporal`: the key module (`%Temporal%`, a method, a tag)...
    const FIRST_KEY: BuiltinModule = BuiltinModule {
        name: "first-key",
        block: 102,
        defs: &[BuiltinDef::method("f", 0, realm_global)],
        intrinsics: &[IntrinsicDef {
            id: IntrinsicId::Temporal,
            value: OBJECT,
        }],
        props: &[
            ObjectProps {
                target: IntrinsicId::Temporal,
                props: &[
                    PropDef::constant(
                        PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                        ConstValue::Str("Temporal"),
                        PropAttrs::CONFIGURABLE,
                    ),
                    PropDef::method(PropKey::Str("f"), 0),
                ],
            },
            ObjectProps {
                target: IntrinsicId::GlobalObject,
                props: &[PropDef::intrinsic(
                    PropKey::Str("Temporal"),
                    IntrinsicId::Temporal,
                    PropAttrs::METHOD,
                )],
            },
        ],
    };

    /// ... and a member module (`%Temporal.Now%` as `Temporal.Now`, an accessor on it).
    const FIRST_MEMBER: BuiltinModule = BuiltinModule {
        name: "first-member",
        block: 103,
        defs: &[BuiltinDef::getter("g", realm_global)],
        intrinsics: &[IntrinsicDef {
            id: IntrinsicId::TemporalNow,
            value: OBJECT,
        }],
        props: &[
            ObjectProps {
                target: IntrinsicId::Temporal,
                props: &[PropDef::intrinsic(
                    PropKey::Str("Now"),
                    IntrinsicId::TemporalNow,
                    PropAttrs::METHOD,
                )],
            },
            ObjectProps {
                target: IntrinsicId::TemporalNow,
                props: &[PropDef::accessor(PropKey::Str("g"), Some(0), None)],
            },
        ],
    };

    /// Group 1, key `Intl` (a synthetic second group): one object with one method.
    const SECOND_KEY: BuiltinModule = BuiltinModule {
        name: "second-key",
        block: 104,
        defs: &[BuiltinDef::method("h", 0, realm_global)],
        intrinsics: &[IntrinsicDef {
            id: IntrinsicId::TemporalDuration,
            value: OBJECT,
        }],
        props: &[
            ObjectProps {
                target: IntrinsicId::TemporalDuration,
                props: &[PropDef::method(PropKey::Str("h"), 0)],
            },
            ObjectProps {
                target: IntrinsicId::GlobalObject,
                props: &[PropDef::intrinsic(
                    PropKey::Str("Intl"),
                    IntrinsicId::TemporalDuration,
                    PropAttrs::METHOD,
                )],
            },
        ],
    };

    /// A key module whose group an eager module names (`%Temporal.Instant%` as a value of
    /// `%Reflect%`): the key stays eager.
    const LEAKED_KEY: BuiltinModule = BuiltinModule {
        name: "leaked-key",
        block: 105,
        defs: &[],
        intrinsics: &[IntrinsicDef {
            id: IntrinsicId::TemporalInstant,
            value: OBJECT,
        }],
        props: &[ObjectProps {
            target: IntrinsicId::GlobalObject,
            props: &[PropDef::intrinsic(
                PropKey::Str("Temporal"),
                IntrinsicId::TemporalInstant,
                PropAttrs::METHOD,
            )],
        }],
    };

    const LEAKER: BuiltinModule = BuiltinModule {
        name: "leaker",
        block: 106,
        defs: &[],
        intrinsics: &[],
        props: &[ObjectProps {
            target: IntrinsicId::Reflect,
            props: &[PropDef::intrinsic(
                PropKey::Str("i"),
                IntrinsicId::TemporalInstant,
                PropAttrs::METHOD,
            )],
        }],
    };

    /// A key module whose key value is a function with a table `name`: the function object
    /// has `name` already (`object::new_function`), which `check_tables` does not see.
    const NAMED_FUNCTION_KEY: BuiltinModule = BuiltinModule {
        name: "named-function-key",
        block: 107,
        defs: &[BuiltinDef::method("T", 0, realm_global)],
        intrinsics: &[IntrinsicDef {
            id: IntrinsicId::Temporal,
            value: IntrinsicValue::Builtin(0),
        }],
        props: &[
            ObjectProps {
                target: IntrinsicId::Temporal,
                props: &[PropDef::constant(
                    PropKey::Str("name"),
                    ConstValue::Str("T2"),
                    PropAttrs::NONE,
                )],
            },
            ObjectProps {
                target: IntrinsicId::GlobalObject,
                props: &[PropDef::intrinsic(
                    PropKey::Str("Temporal"),
                    IntrinsicId::Temporal,
                    PropAttrs::METHOD,
                )],
            },
        ],
    };

    pub(super) static TABLE: BuiltinTable =
        BuiltinTable::new("lazy", &[EAGER, FIRST_KEY, FIRST_MEMBER, SECOND_KEY]);
    pub(super) static NAMED_FUNCTION: BuiltinTable =
        BuiltinTable::new("lazy-named-function", &[EAGER, NAMED_FUNCTION_KEY]);
    pub(super) static LEAKED: BuiltinTable =
        BuiltinTable::new("lazy-leaked", &[EAGER, LEAKED_KEY, LEAKER]);
}

/// The global object's own `key` in the stored form (no swap).
fn raw_global(rt: &Runtime, key: &str) -> Option<(u64, i32)> {
    let k = crate::atoms::intern_str(rt, key);
    object::own_get(rt.global(), k)
}

/// INTL-I1-D8 on a synthetic table with two lazy groups (keys `Temporal`, `Intl`): realm init
/// makes neither group (their intrinsics are undefined in the roots and the REALM record,
/// no group function object exists, each key holds HOLE at its table place with rep ANY, not
/// const); `Cx::has_intrinsic` answers true anyway (M6-ORCH-132); the first read of one key
/// installs only its group, in place (the global object's shape does not move); an intrinsic
/// use (`Cx::function_realm_intrinsic`) installs the other group in a second realm only, with
/// that realm's own function objects; the records stay copies of the roots.
#[test]
fn a_lazy_group_installs_at_first_use_and_alone() {
    use crate::frame::HOLE;
    use crate::object::attr;
    for stress in [false, true] {
        on_big_stack(move || {
            let mut cfg = config(Dispatch::DEFAULT, stress);
            cfg.builtins = Some(&lazy::TABLE);
            let mut iso = isolate(cfg);
            let first = iso.cx.realm;
            let rt: &Runtime = &iso.rt;
            let group_ids = [
                IntrinsicId::Temporal,
                IntrinsicId::TemporalNow,
                IntrinsicId::TemporalDuration,
            ];
            for id in group_ids {
                assert_eq!(intrinsic(rt, id), UNDEFINED, "{} after init", id.name());
                assert_eq!(record_list(first)[id as usize], UNDEFINED, "{}", id.name());
            }
            assert_eq!(builtins::lazy_group(rt, IntrinsicId::Temporal), Some(0));
            assert_eq!(builtins::lazy_group(rt, IntrinsicId::TemporalNow), Some(0));
            assert_eq!(
                builtins::lazy_group(rt, IntrinsicId::TemporalDuration),
                Some(1)
            );
            assert_eq!(builtins::lazy_group(rt, IntrinsicId::Reflect), None);
            assert!(
                object::is_object(intrinsic(rt, IntrinsicId::Reflect)),
                "eager"
            );
            for block in [102, 103, 104] {
                assert_eq!(
                    builtins::existing_function(rt, BuiltinId::new(block, 0)),
                    None,
                    "no group function object after init (block {block})"
                );
            }
            assert!(builtins::existing_function(rt, BuiltinId::new(101, 0)).is_some());
            for key in ["Temporal", "Intl"] {
                assert_eq!(raw_global(rt, key), Some((HOLE, attr::HIDDEN)), "{key}");
                let k = crate::atoms::intern_str(rt, key);
                let s = crate::shape::shape_of(rt.global());
                let f = crate::shape::lookup(rt, s, k).expect("a fast global");
                assert_eq!(
                    crate::shape::field_bits_at(s, f.index),
                    crate::shape::ANY_FIELD,
                    "{key}: rep ANY, not const"
                );
            }
            let shape_before = crate::shape::shape_of(rt.global());
            let r = iso.with_cx(|cx| {
                assert!(cx.has_intrinsic(IntrinsicId::Temporal));
                assert!(cx.has_intrinsic(IntrinsicId::TemporalDuration));
                let g = cx.create_realm()?;
                let rt: &Runtime = cx.inner().rt();
                let second = realm::of_global(rt, g.bits()).expect("a realm's global");
                for id in group_ids {
                    assert_eq!(record_list(second)[id as usize], UNDEFINED, "{}", id.name());
                }
                // The first read of `Temporal` installs group 0 only, in place.
                let global = cx.global();
                let t = cx.get(global, "Temporal")?;
                let rt: &Runtime = cx.inner().rt();
                assert_eq!(t.bits(), intrinsic(rt, IntrinsicId::Temporal));
                assert!(object::is_object(intrinsic(rt, IntrinsicId::TemporalNow)));
                assert_eq!(intrinsic(rt, IntrinsicId::TemporalDuration), UNDEFINED);
                assert!(builtins::existing_function(rt, BuiltinId::new(102, 0)).is_some());
                assert!(builtins::existing_function(rt, BuiltinId::new(103, 0)).is_some());
                assert_eq!(
                    builtins::existing_function(rt, BuiltinId::new(104, 0)),
                    None
                );
                assert_eq!(raw_global(rt, "Temporal"), Some((t.bits(), attr::HIDDEN)));
                assert_eq!(raw_global(rt, "Intl").map(|(v, _)| v), Some(HOLE));
                assert_eq!(crate::shape::shape_of(rt.global()), shape_before);
                assert_eq!(
                    record_list(first),
                    intrinsics(rt),
                    "the record mirrors the roots"
                );
                // The second realm: no install there yet.
                for id in group_ids {
                    assert_eq!(record_list(second)[id as usize], UNDEFINED, "{}", id.name());
                }
                // An intrinsic use with GetFunctionRealm = the second realm installs group 1
                // there only: its own objects and function objects.
                let f = cx.with_realm(g, |cx| {
                    Ok(cx.new_builtin_closure(realm_global, "f", 0, crate::Local::undefined()))
                })?;
                let d = cx.function_realm_intrinsic(f, IntrinsicId::TemporalDuration);
                let rt: &Runtime = cx.inner().rt();
                assert!(object::is_object(d.bits()));
                assert_eq!(
                    record_list(second)[IntrinsicId::TemporalDuration as usize],
                    d.bits()
                );
                assert_eq!(
                    record_list(second)[IntrinsicId::Temporal as usize],
                    UNDEFINED
                );
                assert_eq!(
                    intrinsic(rt, IntrinsicId::TemporalDuration),
                    UNDEFINED,
                    "not in the first"
                );
                assert_eq!(cx.inner().realm, first);
                assert_eq!(record_list(first), intrinsics(rt));
                cx.with_realm(g, |cx| {
                    let rt: &Runtime = cx.inner().rt();
                    assert_eq!(intrinsics(rt), record_list(second));
                    let h = crate::atoms::intern_str(rt, "h");
                    let (hf, _) = object::own_get(d.bits(), h).expect("h");
                    assert_eq!(object::function_realm(hf), second);
                    // The second realm's own function object: not in the module slot.
                    assert_eq!(
                        builtins::existing_function(rt, BuiltinId::new(104, 0)),
                        None
                    );
                    assert_eq!(raw_global(rt, "Intl"), Some((d.bits(), attr::HIDDEN)));
                    assert_eq!(raw_global(rt, "Temporal").map(|(v, _)| v), Some(HOLE));
                    rt.heap.collect();
                    Ok(())
                })?;
                // `Cx::intrinsic` in the first realm installs group 1 there too.
                let d1 = cx.intrinsic(IntrinsicId::TemporalDuration);
                assert_ne!(d1.bits(), d.bits());
                let rt: &Runtime = cx.inner().rt();
                assert!(builtins::existing_function(rt, BuiltinId::new(104, 0)).is_some());
                assert_eq!(raw_global(rt, "Intl").map(|(v, _)| v), Some(d1.bits()));
                Ok(())
            });
            assert_eq!(r, Ok(()));
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

/// A group that a module outside it names (here `%Reflect%.i` = the key's intrinsic) stays
/// eager: its key holds its object from realm init on.
#[test]
fn a_named_group_stays_eager() {
    on_big_stack(|| {
        let mut cfg = config(Dispatch::DEFAULT, false);
        cfg.builtins = Some(&lazy::LEAKED);
        let iso = isolate(cfg);
        let rt: &Runtime = &iso.rt;
        assert_eq!(builtins::lazy_group(rt, IntrinsicId::TemporalInstant), None);
        let v = intrinsic(rt, IntrinsicId::TemporalInstant);
        assert!(object::is_object(v));
        assert_eq!(raw_global(rt, "Temporal").map(|(v, _)| v), Some(v));
    });
}

/// A group whose table defines a key that its function object has already (`name` from
/// `object::new_function`) stays eager (Codex r1 LAZYNS 3): `check_tables` passes the table,
/// and `Isolate::new` reports the install error, as for an eager module, instead of a panic
/// at the first use of the key.
#[test]
fn a_group_with_an_implicit_key_collision_stays_eager() {
    let t: &'static builtins::BuiltinTable = &lazy::NAMED_FUNCTION;
    assert_eq!(
        builtins::check_tables(&[crate::internals::VM_BUILTINS, t]),
        Ok(())
    );
    on_big_stack(move || {
        let mut cfg = config(Dispatch::DEFAULT, false);
        cfg.builtins = Some(t);
        match crate::Isolate::new(cfg) {
            Err(crate::IsolateError::BuiltinTable(e)) => assert!(
                e.contains("%Temporal%.name: the object has this key already"),
                "{e}"
            ),
            Err(e) => panic!("{e}"),
            Ok(_) => panic!("the table was accepted"),
        }
    });
}

/// The raw global read of the tests and of `NativeCx::global_value` (`isolate.rs`
/// `global_data`) installs a lazy group at its first use, as `[[Get]]` does (Codex r1 LAZYNS
/// 1): it gives the namespace object, never HOLE.
#[test]
fn the_raw_global_read_installs_a_lazy_group() {
    on_big_stack(|| {
        let mut cfg = config(Dispatch::DEFAULT, false);
        cfg.builtins = Some(&lazy::TABLE);
        let iso = isolate(cfg);
        let rt: &Runtime = &iso.rt;
        assert_eq!(
            raw_global(rt, "Temporal").map(|(v, _)| v),
            Some(crate::frame::HOLE)
        );
        let v = iso.global_raw("Temporal").expect("a binding").to_bits();
        assert!(object::is_object(v));
        assert_eq!(v, intrinsic(rt, IntrinsicId::Temporal));
        assert_eq!(raw_global(rt, "Temporal").map(|(v, _)| v), Some(v));
    });
}

/// `NativeCx::intrinsic`, the link value of a unit cell "intrinsic by stable index", installs
/// the group of a lazy id first (MC-w30-3; Codex r1 LAZYNS 2): a link before any JS read gets
/// the object that a later JS read gets.
#[cfg(feature = "spike-native")]
#[test]
fn the_native_intrinsic_read_installs_a_lazy_group() {
    on_big_stack(|| {
        let mut cfg = config(Dispatch::DEFAULT, false);
        cfg.builtins = Some(&lazy::TABLE);
        let iso = isolate(cfg);
        let rt: &Runtime = &iso.rt;
        assert_eq!(intrinsic(rt, IntrinsicId::TemporalDuration), UNDEFINED);
        let v = crate::NativeCx::intrinsic(&iso, IntrinsicId::TemporalDuration).to_bits();
        assert!(object::is_object(v));
        assert_eq!(v, intrinsic(rt, IntrinsicId::TemporalDuration));
    });
}
