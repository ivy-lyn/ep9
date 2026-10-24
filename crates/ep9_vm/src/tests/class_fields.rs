//! Tests of class fields, private names and private methods (owner P2-CLS-VM). BytecodeGen
//! emits none of these on this branch (P2-CLS-CG does), so the functions are hand-built with
//! the shapes of the CLS-CG contract (scratchpad p2-clsvm-contract.md) and driven from Rust
//! with `Isolate::call`. Each test runs under every dispatch, with and without GC stress.
//! The JS-level tests are in `crates/ep9_builtins/tests/class_fields.rs`.

use ep9_bytecode::{
    FeedbackSlotKind, FunctionFlags, FunctionId, FunctionKind, FunctionMeta, RegisterList,
    RuntimeId, VerifiedUnit,
};

use super::{Asm, config, isolate, meta, on_big_stack, unit};
use crate::frame::UNDEFINED;
use crate::isolate::Isolate;
use crate::object::{self, attr};
use crate::runtime::{Atom, val};
use crate::{Dispatch, Value, atoms, property, props, symbol};

/// The test functions, in function id order (id 0 is the script that makes them globals,
/// except the inner functions `anonFn` and `namedFn`, which follow their parents: function ids
/// are a preorder of the parent links).
const FNS: &[&str] = &[
    "mk",
    "def",
    "def0",
    "defName",
    "get",
    "set",
    "has",
    "setInit",
    "runInit",
    "pub",
    "pubName",
    "litName",
    "anon",
    "anonFn",
    "named",
    "namedFn",
    "init",
    "A",
    "B",
    "D",
    "throwInit",
];

fn id(name: &str) -> u32 {
    FNS.iter().position(|&n| n == name).unwrap() as u32 + 1
}

fn fn_meta(name: &str, params: u32) -> FunctionMeta {
    meta(name, params, 0, 0, 0)
}

/// `CallRuntime(rid, params 0..argc)`; returns the result.
fn runtime_fn(name: &str, rid: RuntimeId, argc: u32) -> Asm {
    let mut s = Asm::new(fn_meta(name, argc));
    let list = s.b.new_register_list(argc);
    for i in 0..argc {
        let p = s.b.parameter(i);
        s.b.mov(p, list.get(i));
    }
    s.b.call_runtime(rid, list, argc).return_();
    s
}

/// `(o, k, v) => DefineKeyedOwnProperty o, k, flags` (acc = v); returns undefined.
fn define_fn(name: &str, flags: u8, literal: bool) -> Asm {
    let mut s = Asm::new(fn_meta(name, 3));
    let (o, k, v) = (s.b.parameter(0), s.b.parameter(1), s.b.parameter(2));
    let slot = s.b.feedback_slot(FeedbackSlotKind::DefineKeyedOwn);
    s.b.ldar(v);
    if literal {
        s.b.define_keyed_own_property_in_literal(o, k, flags, slot);
    } else {
        s.b.define_keyed_own_property(o, k, flags, slot);
    }
    s.b.lda_undefined().return_();
    s
}

/// A class constructor whose body is the base-constructor prologue of the contract:
/// `RunClassFieldsInitializer(<current function>, this)`, then `this` is returned.
fn class_ctor(name: &str, kind: FunctionKind) -> Asm {
    let mut m = fn_meta(name, 0);
    m.kind = kind;
    m.flags = FunctionFlags::STRICT | FunctionFlags::SIMPLE_PARAMETERS;
    let mut s = Asm::new(m);
    let t = s.b.new_register_list(2);
    let this = s.b.receiver();
    s.b.lda_current_function().star(t.get(0));
    s.b.ldar(this).star(t.get(1));
    s.b.call_runtime(RuntimeId::RunClassFieldsInitializer, t, 2);
    s.b.lda_undefined().return_();
    s
}

fn make() -> VerifiedUnit {
    let mut fns = vec![
        runtime_fn("mk", RuntimeId::CreatePrivateName, 1),
        define_fn("def", 0x02, false),
        define_fn("def0", 0x00, false),
        define_fn("defName", 0x03, false),
    ];
    // get(o, k) = o[k]
    let mut g = Asm::new(fn_meta("get", 2));
    let (o, k) = (g.b.parameter(0), g.b.parameter(1));
    g.b.ldar(k).get_keyed_property(o, g.keyed).return_();
    fns.push(g);
    // set(o, k, v): o[k] = v; returns v
    let mut st = Asm::new(fn_meta("set", 3));
    let (o, k, v) = (st.b.parameter(0), st.b.parameter(1), st.b.parameter(2));
    st.b.ldar(v)
        .set_keyed_property(o, k, st.set_keyed)
        .return_();
    fns.push(st);
    // has(o, k) = k in o
    let mut h = Asm::new(fn_meta("has", 2));
    let (o, k) = (h.b.parameter(0), h.b.parameter(1));
    let hs = h.b.feedback_slot(FeedbackSlotKind::HasKeyed);
    h.b.ldar(o).test_in(k, hs).return_();
    fns.push(h);
    fns.push(runtime_fn(
        "setInit",
        RuntimeId::SetClassFieldsInitializer,
        2,
    ));
    fns.push(runtime_fn(
        "runInit",
        RuntimeId::RunClassFieldsInitializer,
        2,
    ));
    fns.push(define_fn("pub", 0x00, false));
    fns.push(define_fn("pubName", 0x01, false));
    fns.push(define_fn("litName", 0x01, true));
    // anon(): a new anonymous closure of `anonFn` each call.
    let mut a = Asm::new(fn_meta("anon", 0));
    let ka = a.b.function_constant(FunctionId(id("anonFn")));
    a.b.create_closure(ka, a.closure, 0).return_();
    fns.push(a);
    // anonFn (a child of `anon`): no name in the unit.
    let mut am = meta("", 0, id("anon"), 0, 0);
    am.name = None;
    let mut af = Asm::new(am);
    af.b.lda_smi(1).return_();
    fns.push(af);
    // named(): a new closure of the named function `namedFn` each call.
    let mut n = Asm::new(fn_meta("named", 0));
    let kn = n.b.function_constant(FunctionId(id("namedFn")));
    n.b.create_closure(kn, n.closure, 0).return_();
    fns.push(n);
    let mut nf = Asm::new(meta("namedFn", 0, id("named"), 0, 0));
    nf.b.lda_smi(2).return_();
    fns.push(nf);
    // init(): `this.x = 7` (DefineNamedOwnProperty, as a field initializer), returns 99.
    let mut i = Asm::new(fn_meta("init", 0));
    let this = i.b.receiver();
    let kx = i.b.string_constant("x");
    let ds = i.b.feedback_slot(FeedbackSlotKind::DefineNamedOwn);
    i.b.lda_smi(7).define_named_own_property(this, kx, ds);
    i.b.lda_smi(99).return_();
    fns.push(i);
    fns.push(class_ctor("A", FunctionKind::ClassConstructor));
    fns.push(class_ctor("B", FunctionKind::ClassConstructor));
    fns.push(class_ctor("D", FunctionKind::ClassConstructor));
    // throwInit(): throws 13.
    let mut ti = Asm::new(fn_meta("throwInit", 0));
    ti.b.lda_smi(13).throw();
    fns.push(ti);

    // The script: every function of FNS is a global; A and B are classes (DefineClass); D
    // extends A.
    let mut s = Asm::script(0);
    let (cls, heritage) = (s.b.new_register(), s.b.new_register());
    let def = RegisterList::new(cls.local_index().unwrap(), 2);
    for (j, name) in FNS.iter().enumerate() {
        if name.ends_with("Fn") {
            continue;
        }
        let k = s.b.function_constant(FunctionId(j as u32 + 1));
        s.b.create_closure(k, s.closure, 0);
        match *name {
            "A" | "B" => {
                s.b.star(cls).lda_the_hole().star(heritage);
                s.b.call_runtime(RuntimeId::DefineClass, def, 2);
                s.b.ldar(cls);
            }
            "D" => {
                s.b.star(cls);
                s.lda_global("A");
                s.b.star(heritage);
                s.b.call_runtime(RuntimeId::DefineClass, def, 2);
                s.b.ldar(cls);
            }
            _ => {}
        }
        s.sta_global(name);
    }
    s.b.lda_undefined().return_();
    let mut all = vec![s.finish()];
    all.extend(fns.into_iter().map(Asm::finish));
    unit("", all)
}

/// Runs `test` on a fresh isolate (with the globals of [`make`]) under every dispatch, with and
/// without GC stress.
fn each(test: fn(&mut Isolate)) {
    assert_eq!(id("mk"), 1);
    for &d in Dispatch::ALL {
        for stress in [false, true] {
            on_big_stack(move || {
                let mut iso = isolate(config(d, stress));
                iso.with_cx_checkpoint(|cx| cx.run_script(make(), "class_fields.js").map(|_| ()))
                    .unwrap();
                test(&mut iso);
                iso.collect_garbage();
                iso.verify_heap().unwrap();
            });
        }
    }
}

/// `name(args)` with `this` undefined.
fn call(iso: &mut Isolate, name: &str, args: &[Value]) -> Result<Value, String> {
    let f = iso.global_raw(name).unwrap();
    iso.call_raw(f, Value::UNDEFINED, args)
        .map_err(|e| e.message)
}

fn ok(iso: &mut Isolate, name: &str, args: &[Value]) -> Value {
    call(iso, name, args).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn err(iso: &mut Isolate, name: &str, args: &[Value]) -> String {
    match call(iso, name, args) {
        Ok(v) => panic!("{name} returned {}", iso.display_raw(v)),
        Err(e) => e,
    }
}

fn s(iso: &Isolate, text: &str) -> Value {
    val(atoms::intern_str(&iso.rt, text))
}

fn obj(iso: &Isolate) -> Value {
    val(object::new_plain_object(&iso.rt))
}

fn int(n: i32) -> Value {
    Value::number(f64::from(n))
}

/// The own property keys of `o` as Rust text (symbols as `Symbol(desc)`).
fn own_keys(iso: &mut Isolate, o: Value) -> Vec<String> {
    let keys = property::own_property_keys(&mut iso.cx, o.to_bits());
    (0..object::array_len(keys))
        .map(|i| {
            let k = object::array_get(keys, i).unwrap();
            if crate::kinds::is_symbol(k) {
                symbol::descriptive_text(k)
            } else {
                crate::string::to_rust_string(k)
            }
        })
        .collect()
}

/// The initializer stored on `ctor` under the class-fields key, with its attributes 0.
fn stored_initializer(iso: &Isolate, ctor: Value) -> Option<u64> {
    let key = symbol::class_fields_key(&iso.rt);
    assert!(symbol::is_private_key(key));
    props::own_get(ctor.to_bits(), key).map(|(f, a)| {
        assert_eq!(a, 0);
        f
    })
}

fn function_name(iso: &Isolate, f: Value) -> String {
    let (n, _) = props::own_get(f.to_bits(), iso.rt.atom(Atom::Name)).unwrap();
    crate::string::to_rust_string(n)
}

#[test]
fn create_private_name_makes_a_new_private_symbol_each_call() {
    each(|iso| {
        let d = s(iso, "#x");
        let a = ok(iso, "mk", &[d]).to_bits();
        let b = ok(iso, "mk", &[d]).to_bits();
        assert!(symbol::is_private_key(a) && symbol::is_private_key(b));
        assert_ne!(a, b, "NewPrivateName is a new name every time");
        assert_eq!(symbol::description(a), d.to_bits());
        let u = ok(iso, "mk", &[Value::UNDEFINED]).to_bits();
        assert!(symbol::is_private_key(u) && symbol::description(u) == UNDEFINED);
        assert_eq!(
            err(iso, "mk", &[int(1)]),
            "Error: ep9 internal error: CreatePrivateName description is not a string"
        );
    });
}

#[test]
fn private_fields_add_get_set_and_in() {
    each(|iso| {
        let d = s(iso, "#x");
        let k = ok(iso, "mk", &[d]);
        let o = obj(iso);
        let other = obj(iso);
        ok(iso, "def", &[o, k, int(1)]);
        assert_eq!(ok(iso, "get", &[o, k]).as_int32(), Some(1));
        assert_eq!(ok(iso, "set", &[o, k, int(2)]).as_int32(), Some(2));
        assert_eq!(ok(iso, "get", &[o, k]).as_int32(), Some(2));
        assert_eq!(ok(iso, "has", &[o, k]).to_bits(), Value::TRUE.to_bits());
        assert_eq!(
            ok(iso, "has", &[other, k]).to_bits(),
            Value::FALSE.to_bits()
        );
        // Not a property: no key lists it.
        assert!(own_keys(iso, o).is_empty(), "{:?}", own_keys(iso, o));
        // A second add, and the other errors (node v24 texts).
        assert_eq!(
            err(iso, "def", &[o, k, int(3)]),
            "TypeError: Cannot initialize #x twice on the same object"
        );
        assert_eq!(ok(iso, "get", &[o, k]).as_int32(), Some(2));
        let read = "TypeError: Cannot read private member #x from an object whose class did not declare it";
        let write =
            "TypeError: Cannot write private member #x to an object whose class did not declare it";
        assert_eq!(err(iso, "get", &[other, k]), read);
        assert_eq!(err(iso, "set", &[other, k, int(1)]), write);
        assert_eq!(err(iso, "get", &[int(1), k]), read);
        assert_eq!(
            err(iso, "get", &[Value::UNDEFINED, k]),
            "TypeError: Cannot read properties of undefined (reading '#x')"
        );
        assert_eq!(
            err(iso, "set", &[Value::NULL, k, int(1)]),
            "TypeError: Cannot set properties of null (setting '#x')"
        );
        assert_eq!(
            err(iso, "has", &[int(1), k]),
            "TypeError: Cannot use 'in' operator to search for '#x' in 1"
        );
        assert_eq!(
            err(iso, "def", &[int(1), k, int(1)]),
            "TypeError: Cannot define a property on a non-object"
        );
        // Own lookup only: an object whose prototype has #x does not have it.
        let child = val(object::new_object(&iso.rt, o.to_bits()));
        assert_eq!(err(iso, "get", &[child, k]), read);
        assert_eq!(
            ok(iso, "has", &[child, k]).to_bits(),
            Value::FALSE.to_bits()
        );
        // Flags without bit 1 still add a private key (the add is the only own define of a
        // private name).
        ok(iso, "def0", &[child, k, int(5)]);
        assert_eq!(ok(iso, "get", &[child, k]).as_int32(), Some(5));
        assert_eq!(ok(iso, "get", &[o, k]).as_int32(), Some(2));
        // Two names with the same description are different names.
        let k2 = ok(iso, "mk", &[d]);
        assert_eq!(ok(iso, "has", &[o, k2]).to_bits(), Value::FALSE.to_bits());
        ok(iso, "def", &[o, k2, int(9)]);
        assert_eq!(ok(iso, "get", &[o, k]).as_int32(), Some(2));
        assert_eq!(ok(iso, "get", &[o, k2]).as_int32(), Some(9));
    });
}

/// Acceptance (D4, M2-PRIVATE-EXT): after `property::prevent_extensions`, a
/// DefineKeyedOwnProperty with flags 0x02 and a private key throws a TypeError; on an
/// extensible object it adds the key, and a second add throws.
#[test]
fn private_add_on_a_non_extensible_object_throws() {
    each(|iso| {
        let dx = s(iso, "#x");
        let k = ok(iso, "mk", &[dx]);
        let dc = s(iso, "C");
        let brand = ok(iso, "mk", &[dc]);
        let sealed = obj(iso);
        assert!(property::prevent_extensions(&iso.rt, sealed.to_bits()));
        assert_eq!(
            err(iso, "def", &[sealed, k, int(1)]),
            "TypeError: Cannot define private field #x on a non-extensible object"
        );
        assert_eq!(
            err(iso, "def", &[sealed, brand, Value::UNDEFINED]),
            "TypeError: Cannot define private methods of class C on a non-extensible object"
        );
        assert_eq!(
            ok(iso, "has", &[sealed, k]).to_bits(),
            Value::FALSE.to_bits()
        );
        // Extensible: the add works once.
        let o = obj(iso);
        ok(iso, "def", &[o, k, int(1)]);
        assert_eq!(
            err(iso, "def", &[o, k, int(1)]),
            "TypeError: Cannot initialize #x twice on the same object"
        );
        // A field added before preventExtensions stays readable and writable.
        assert!(property::prevent_extensions(&iso.rt, o.to_bits()));
        assert_eq!(ok(iso, "set", &[o, k, int(4)]).as_int32(), Some(4));
        assert_eq!(ok(iso, "get", &[o, k]).as_int32(), Some(4));
    });
}

#[test]
fn brand_errors_name_the_class() {
    each(|iso| {
        let dc = s(iso, "C");
        let brand = ok(iso, "mk", &[dc]);
        let empty = s(iso, "");
        let anon_brand = ok(iso, "mk", &[empty]);
        let o = obj(iso);
        assert_eq!(
            err(iso, "get", &[o, brand]),
            "TypeError: Receiver must be an instance of class C"
        );
        assert_eq!(
            err(iso, "get", &[o, anon_brand]),
            "TypeError: Receiver must be an instance of class anonymous"
        );
        ok(iso, "def", &[o, brand, Value::UNDEFINED]);
        assert_eq!(
            ok(iso, "get", &[o, brand]).to_bits(),
            Value::UNDEFINED.to_bits()
        );
        assert_eq!(ok(iso, "has", &[o, brand]).to_bits(), Value::TRUE.to_bits());
        assert_eq!(
            err(iso, "def", &[o, brand, Value::UNDEFINED]),
            "TypeError: Cannot initialize private methods of class C twice on the same object"
        );
    });
}

#[test]
fn class_fields_initializer_is_stored_hidden_and_run_own_only() {
    each(|iso| {
        let (a, b, d) = (
            iso.global_raw("A").unwrap(),
            iso.global_raw("B").unwrap(),
            iso.global_raw("D").unwrap(),
        );
        let init = iso.global_raw("init").unwrap();
        let keys_before = own_keys(iso, a);
        assert_eq!(
            ok(iso, "setInit", &[a, init]).to_bits(),
            Value::UNDEFINED.to_bits()
        );
        // Hidden: [[OwnPropertyKeys]] of A does not change (it lists `length`, `name`,
        // `prototype` only).
        assert_eq!(own_keys(iso, a), keys_before);
        assert_eq!(keys_before, ["length", "name", "prototype"]);
        // MakeMethod: [[HomeObject]] is A.prototype.
        let (proto, _) = props::own_get(a.to_bits(), iso.rt.atom(Atom::Prototype)).unwrap();
        assert_eq!(object::home_object(init.to_bits()), proto);
        assert_eq!(stored_initializer(iso, a), Some(init.to_bits()));
        // Run: Call(init, receiver); the result is undefined, not init's 99.
        let o = obj(iso);
        assert_eq!(
            ok(iso, "runInit", &[a, o]).to_bits(),
            Value::UNDEFINED.to_bits()
        );
        let kx = s(iso, "x");
        assert_eq!(
            props::own_get(o.to_bits(), kx.to_bits()).map(|(v, _)| v),
            Some(int(7).to_bits())
        );
        // No initializer: nothing runs. D extends A: A's initializer is not D's (own only).
        let o2 = obj(iso);
        ok(iso, "runInit", &[b, o2]);
        ok(iso, "runInit", &[d, o2]);
        ok(iso, "runInit", &[Value::UNDEFINED, o2]);
        assert!(props::own_get(o2.to_bits(), kx.to_bits()).is_none());
        // An undefined initializer stores nothing.
        ok(iso, "setInit", &[b, Value::UNDEFINED]);
        assert_eq!(stored_initializer(iso, b), None);
        // `new A()`: the constructor prologue runs the fields; `new D()` (no super() in this
        // hand-built body) does not run A's.
        let mut cx = crate::abi::Cx::new(&mut iso.cx);
        let al = crate::abi::Local::<crate::abi::JsValue>::from_bits(a.to_bits());
        let inst = cx.construct(al, &[], al).unwrap();
        let x = cx.get(inst.as_object().unwrap(), "x").unwrap();
        assert_eq!(x.as_int32(), Some(7));
        // An exception from the initializer propagates.
        let t = iso.global_raw("throwInit").unwrap();
        ok(iso, "setInit", &[b, t]);
        assert_eq!(err(iso, "runInit", &[b, o2]), "13");
        // Errors of the runtime function itself.
        assert_eq!(
            err(iso, "setInit", &[o2, init]),
            "Error: ep9 internal error: SetClassFieldsInitializer needs two functions"
        );
    });
}

#[test]
fn bit_0_names_an_anonymous_function_from_the_key() {
    each(|iso| {
        // A string key; the literal opcode too.
        let o = obj(iso);
        let f = ok(iso, "anon", &[]);
        assert_eq!(function_name(iso, f), "");
        let kk = s(iso, "k");
        ok(iso, "pubName", &[o, kk, f]);
        assert_eq!(function_name(iso, f), "k");
        let f2 = ok(iso, "anon", &[]);
        let l = s(iso, "lit");
        ok(iso, "litName", &[o, l, f2]);
        assert_eq!(function_name(iso, f2), "lit");
        let (got, a) = props::own_get(o.to_bits(), l.to_bits()).unwrap();
        assert_eq!((got, a), (f2.to_bits(), attr::DEFAULT));
        // A number key goes through ToPropertyKey once.
        let f3 = ok(iso, "anon", &[]);
        ok(iso, "pubName", &[o, int(1), f3]);
        assert_eq!(function_name(iso, f3), "1");
        // Symbol keys: "[desc]", or "" without a description.
        let sd = s(iso, "d");
        let sym = val(symbol::new_symbol(&iso.rt, sd.to_bits()));
        let f4 = ok(iso, "anon", &[]);
        ok(iso, "pubName", &[o, sym, f4]);
        assert_eq!(function_name(iso, f4), "[d]");
        let sym2 = val(symbol::new_symbol(&iso.rt, UNDEFINED));
        let f5 = ok(iso, "anon", &[]);
        ok(iso, "pubName", &[o, sym2, f5]);
        assert_eq!(function_name(iso, f5), "");
        // Without bit 0 the name stays "".
        let f6 = ok(iso, "anon", &[]);
        let kq = s(iso, "q");
        ok(iso, "pub", &[o, kq, f6]);
        assert_eq!(function_name(iso, f6), "");
        // A named function keeps its name; a function whose `name` changed keeps it (a class
        // with `static name() {}`); a non-function value is just defined.
        let n = ok(iso, "named", &[]);
        ok(iso, "pubName", &[o, kk, n]);
        assert_eq!(function_name(iso, n), "namedFn");
        let f7 = ok(iso, "anon", &[]);
        let other = s(iso, "other");
        props::own_define(
            &iso.rt,
            f7.to_bits(),
            iso.rt.atom(Atom::Name),
            other.to_bits(),
            attr::HIDDEN,
        );
        ok(iso, "pubName", &[o, kk, f7]);
        assert_eq!(function_name(iso, f7), "other");
        ok(iso, "pubName", &[o, kk, int(3)]);
        // A private key with bit 0: the description.
        let dx = s(iso, "#x");
        let k = ok(iso, "mk", &[dx]);
        let f8 = ok(iso, "anon", &[]);
        ok(iso, "defName", &[o, k, f8]);
        assert_eq!(function_name(iso, f8), "#x");
        assert_eq!(ok(iso, "get", &[o, k]).to_bits(), f8.to_bits());
        // A failed define on a non-object throws before naming.
        let f9 = ok(iso, "anon", &[]);
        assert_eq!(
            err(iso, "pubName", &[int(1), kk, f9]),
            "TypeError: Cannot define a property on a non-object"
        );
        assert_eq!(function_name(iso, f9), "");
    });
}
