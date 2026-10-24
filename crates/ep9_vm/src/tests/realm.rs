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
