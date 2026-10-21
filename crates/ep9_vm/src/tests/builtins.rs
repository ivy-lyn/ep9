//! Builtin tables and realm init (H20): the VM table, table checks, identity and aliases,
//! property order, the core intrinsics, `Function`, `Object(v)`, the P3 ABI surface, and the
//! global declaration checks of `DeclareGlobals` (§16.1.7).

use ep9_bytecode::{FunctionId, RegisterList, RuntimeId, ScopeKind, VariableMode, VerifiedUnit};

use super::{Asm, config, isolate, meta, on_big_stack, unit_with_scopes};
use crate::builtins::{self, BuiltinState};
use crate::runtime::Runtime;
use crate::{
    Args, BuiltinDef, BuiltinId, BuiltinModule, BuiltinTable, ConstValue, Cx, Dispatch,
    IntrinsicDef, IntrinsicId, IntrinsicValue, Isolate, IsolateConfig, IsolateError, JsResult,
    Local, ObjectProps, PrimitiveType, PropAttrs, PropDef, PropKey, PropValue, PropertyDescriptor,
    WellKnownSymbol, check_tables,
};

// ---- helpers --------------------------------------------------------------------------------

/// Runs `f` with the builtin-ABI context of a fresh isolate (`table` after the VM's), then
/// collects and verifies the heap. `stress`: a collection before every allocation.
fn with_table<R>(
    table: Option<&'static BuiltinTable>,
    stress: bool,
    f: impl FnOnce(&mut Cx<'_>, &Runtime) -> R,
) -> R {
    let mut iso = isolate(IsolateConfig {
        builtins: table,
        ..config(Dispatch::DEFAULT, stress)
    });
    let out = {
        let iso = &mut iso;
        let mut cx = Cx::new(&mut iso.cx);
        f(&mut cx, &iso.rt)
    };
    iso.collect_garbage();
    iso.verify_heap().unwrap();
    out
}

fn global<'s>(cx: &mut Cx<'s>, name: &str) -> Local<'s> {
    let g = cx.global();
    cx.get(g, name).unwrap()
}

/// "Name: message" of the pending exception (taken).
fn take_error(cx: &mut Cx<'_>, rt: &Runtime) -> String {
    let e: Local<'_> = Local::from_bits(rt.take_pending());
    let e = e.as_object().expect("an Error object");
    let name = cx.get(e, "name").unwrap();
    let msg = cx.get(e, "message").unwrap();
    format!(
        "{}: {}",
        cx.to_rust_string(name).unwrap(),
        cx.to_rust_string(msg).unwrap()
    )
}

/// The own keys of `o` as text, in [[OwnPropertyKeys]] order.
fn keys<'s>(cx: &mut Cx<'s>, rt: &Runtime, o: Local<'s, crate::JsObject>) -> Vec<String> {
    let a = cx.own_property_keys(o).unwrap();
    (0..crate::object::array_len(a.bits()))
        .map(|i| {
            let k = cx.get(a, i.to_string().as_str()).unwrap();
            crate::key::key_text(rt, k.bits())
        })
        .collect()
}

/// Every installed intrinsic with its own keys.
fn intrinsic_keys(cx: &mut Cx<'_>, rt: &Runtime) -> Vec<(IntrinsicId, Vec<String>)> {
    let mut out = Vec::new();
    for id in IntrinsicId::ALL {
        if cx.has_intrinsic(id) {
            let o = cx.intrinsic(id);
            out.push((id, keys(cx, rt, o)));
        }
    }
    out
}

// ---- a test table ---------------------------------------------------------------------------

fn answer<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::int32(42))
}

fn count_args<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::int32(args.len() as i32))
}

/// A namespace object `%Math%` (the slot a module may fill) with a method, an alias of it, a
/// constant and a string; `Thing` is a constructor on the global object.
const TEST_DEFS: &[BuiltinDef] = &[
    BuiltinDef::method("answer", 0, answer),
    BuiltinDef::constructor("Thing", 2, count_args),
];

const TEST_MODULE: BuiltinModule = BuiltinModule {
    name: "test",
    block: 100,
    defs: TEST_DEFS,
    intrinsics: &[IntrinsicDef {
        id: IntrinsicId::Math,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    }],
    props: &[
        ObjectProps {
            target: IntrinsicId::Math,
            props: &[
                PropDef::method(PropKey::Str("answer"), 0),
                PropDef::alias(
                    PropKey::Str("same"),
                    PropKey::Str("answer"),
                    PropAttrs::NONE,
                ),
                PropDef::constant(
                    PropKey::Str("PI"),
                    ConstValue::Number(std::f64::consts::PI),
                    PropAttrs::NONE,
                ),
                PropDef::constant(PropKey::Str("tag"), ConstValue::Str("Math"), PropAttrs::ALL),
            ],
        },
        ObjectProps {
            target: IntrinsicId::GlobalObject,
            props: &[
                PropDef::intrinsic(PropKey::Str("Math"), IntrinsicId::Math, PropAttrs::METHOD),
                PropDef::method(PropKey::Str("Thing"), 1),
            ],
        },
    ],
};

static TEST_TABLE: BuiltinTable = BuiltinTable::new("test", &[TEST_MODULE]);

// ---- the VM table and realm init ---------------------------------------------------------------

#[test]
fn the_vm_table_and_the_test_table_pass_the_checks() {
    let vm = crate::internals::VM_BUILTINS;
    assert_eq!(check_tables(&[vm]), Ok(()));
    assert_eq!(check_tables(&[vm, &TEST_TABLE]), Ok(()));
    assert_eq!(builtins::check(None), Ok(()));
    assert_eq!(builtins::check(Some(&TEST_TABLE)), Ok(()));
}

#[test]
fn two_isolates_have_the_same_intrinsic_property_order() {
    for table in [None, Some(&TEST_TABLE)] {
        let a = with_table(table, false, intrinsic_keys);
        let b = with_table(table, true, intrinsic_keys);
        assert_eq!(a, b);
        assert!(a.len() >= 26, "core and VM intrinsics: {}", a.len());
    }
    // The week-3 order of the global object, then `Function` (the test table appends).
    with_table(Some(&TEST_TABLE), false, |cx, rt| {
        let g = cx.global();
        assert_eq!(
            keys(cx, rt, g),
            [
                "globalThis",
                "undefined",
                "NaN",
                "Infinity",
                "Object",
                "Array",
                "Error",
                "TypeError",
                "RangeError",
                "ReferenceError",
                "SyntaxError",
                "EvalError",
                "URIError",
                "Function",
                "Math",
                "Thing",
                "print",
            ]
        );
        let op = cx.intrinsic(IntrinsicId::ObjectPrototype);
        assert_eq!(keys(cx, rt, op), ["toString", "valueOf", "constructor"]);
        let ep = cx.intrinsic(IntrinsicId::ErrorPrototype);
        assert_eq!(
            keys(cx, rt, ep),
            ["toString", "name", "message", "constructor"]
        );
    });
}

#[test]
fn a_prop_def_alias_gives_the_same_function_object() {
    with_table(Some(&TEST_TABLE), true, |cx, rt| {
        let math = global(cx, "Math").as_object().unwrap();
        assert!(math.is_same_bits(&cx.intrinsic(IntrinsicId::Math)));
        let a = cx.get(math, "answer").unwrap();
        let b = cx.get(math, "same").unwrap();
        assert!(a.is_callable() && a.is_same_bits(&b));
        // The alias has its own attributes; the function is the builtin of id 100.0.
        let d = cx.get_own_property(math, "same").unwrap().unwrap();
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(false), Some(false), Some(false))
        );
        let f = builtins::existing_function(rt, BuiltinId::new(100, 0)).unwrap();
        assert_eq!(f, a.bits());
        assert_eq!(
            cx.call(a, Local::undefined(), &[]).unwrap().as_int32(),
            Some(42)
        );
        let name = cx.get(a.as_object().unwrap(), "name").unwrap();
        assert_eq!(cx.to_rust_string(name).unwrap(), "answer");
        let pi = cx.get(math, "PI").unwrap();
        assert_eq!(pi.as_number(), Some(std::f64::consts::PI));
        let tag = cx.get(math, "tag").unwrap();
        assert_eq!(cx.to_rust_string(tag).unwrap(), "Math");
        // A table constructor: [[Construct]] and `length` from the definition.
        let thing = global(cx, "Thing");
        assert!(cx.is_constructor(thing));
        let len = cx.get(thing.as_object().unwrap(), "length").unwrap();
        assert_eq!(len.as_int32(), Some(2));
        let n = cx
            .call(
                thing,
                Local::undefined(),
                &[Local::int32(1), Local::int32(2)],
            )
            .unwrap();
        assert_eq!(n.as_int32(), Some(2));
    });
}

#[test]
fn one_function_object_per_builtin_and_dense_infos() {
    with_table(None, false, |cx, rt| {
        let object = global(cx, "Object").as_object().unwrap();
        let op = cx.intrinsic(IntrinsicId::ObjectPrototype);
        let ctor = cx.get(op, "constructor").unwrap();
        assert!(ctor.is_same_bits(&object));
        assert!(object.is_same_bits(&cx.intrinsic(IntrinsicId::Object)));
        let proto = cx.get(object, "prototype").unwrap();
        assert!(proto.is_same_bits(&op));
        // Builtins live in the dense per-module arrays, not in `rt.infos` (only `print`).
        assert_eq!(rt.infos.borrow().len(), 1);
        let vm = crate::internals::VM_BUILTINS.modules[0];
        for i in 0..vm.defs.len() as u16 {
            let f = builtins::existing_function(rt, vm.id(i)).expect("made at init");
            assert!(crate::kinds::is_function(f));
            assert!(crate::object::function_info(f).slot.kind().is_none());
        }
        let state: &BuiltinState = &rt.builtins;
        assert!(state.table().is_none());
    });
}

#[test]
fn core_intrinsics_and_their_prototypes() {
    with_table(None, true, |cx, _| {
        let op = cx.intrinsic(IntrinsicId::ObjectPrototype);
        assert!(cx.get_prototype_of(op).unwrap().is_null());
        for id in [
            IntrinsicId::FunctionPrototype,
            IntrinsicId::ArrayPrototype,
            IntrinsicId::StringPrototype,
            IntrinsicId::NumberPrototype,
            IntrinsicId::BooleanPrototype,
            IntrinsicId::ErrorPrototype,
            IntrinsicId::IteratorPrototype,
            IntrinsicId::GlobalObject,
        ] {
            let o = cx.intrinsic(id);
            let p = cx.get_prototype_of(o).unwrap();
            assert!(p.is_same_bits(&op), "{}", id.name());
        }
        let ip = cx.intrinsic(IntrinsicId::IteratorPrototype);
        let aip = cx.intrinsic(IntrinsicId::ArrayIteratorPrototype);
        assert!(cx.get_prototype_of(aip).unwrap().is_same_bits(&ip));
        let ep = cx.intrinsic(IntrinsicId::ErrorPrototype);
        let tep = cx.intrinsic(IntrinsicId::TypeErrorPrototype);
        assert!(cx.get_prototype_of(tep).unwrap().is_same_bits(&ep));
        // Every constructor of the VM table is a function whose [[Prototype]] is
        // %Function.prototype%.
        let fp = cx.intrinsic(IntrinsicId::FunctionPrototype);
        for id in [
            IntrinsicId::Object,
            IntrinsicId::Function,
            IntrinsicId::TypeError,
        ] {
            let c = cx.intrinsic(id);
            assert!(cx.is_constructor(c));
            assert!(cx.get_prototype_of(c).unwrap().is_same_bits(&fp));
        }
        // Tables made none of these; %Symbol.prototype% comes with symbols (P2-SYM).
        assert!(!cx.has_intrinsic(IntrinsicId::Math));
        assert!(!cx.has_intrinsic(IntrinsicId::Number));
    });
}

#[test]
fn function_prototype_is_callable_and_function_throws() {
    with_table(None, true, |cx, rt| {
        let fp = cx.intrinsic(IntrinsicId::FunctionPrototype);
        assert_eq!(cx.type_of(fp), "function");
        assert!(cx.is_callable(fp) && !cx.is_constructor(fp));
        let r = cx.call(fp, Local::int32(1), &[Local::int32(2)]).unwrap();
        assert!(r.is_undefined());
        let len = cx.get(fp, "length").unwrap();
        assert_eq!(len.as_int32(), Some(0));
        let name = cx.get(fp, "name").unwrap();
        assert_eq!(cx.to_rust_string(name).unwrap(), "");
        let function = global(cx, "Function");
        assert!(function.is_same_bits(&cx.get(fp, "constructor").unwrap()));
        let p = cx.get(function.as_object().unwrap(), "prototype").unwrap();
        assert!(p.is_same_bits(&fp));
        assert!(cx.call(function, Local::undefined(), &[]).is_err());
        assert_eq!(
            take_error(cx, rt),
            "Error: not implemented in ep9 v0: Function constructor"
        );
        assert!(cx.construct(function, &[], function).is_err());
        assert_eq!(
            take_error(cx, rt),
            "Error: not implemented in ep9 v0: Function constructor"
        );
    });
}

#[test]
fn object_constructor_uses_new_target_and_to_object() {
    with_table(None, true, |cx, rt| {
        let object = global(cx, "Object");
        let op = cx.intrinsic(IntrinsicId::ObjectPrototype);
        // Object(), Object(undefined), Object(null), new Object(): a new plain object.
        for args in [&[][..], &[Local::undefined()], &[Local::null()]] {
            let o = cx.call(object, Local::undefined(), args).unwrap();
            let o = o.as_object().unwrap();
            assert!(cx.get_prototype_of(o).unwrap().is_same_bits(&op));
        }
        let o = cx.construct(object, &[], object).unwrap();
        assert!(cx.get_prototype_of(o).unwrap().is_same_bits(&op));
        // Object(o) is o.
        let x = cx.new_array();
        let r = cx.call(object, Local::undefined(), &[x.value()]).unwrap();
        assert!(r.is_same_bits(&x));
        // A NewTarget other than Object: OrdinaryCreateFromConstructor (`class X extends
        // Object`).
        let te = global(cx, "TypeError");
        let made = cx.construct(object, &[], te).unwrap();
        let tep = cx.intrinsic(IntrinsicId::TypeErrorPrototype);
        assert!(cx.get_prototype_of(made).unwrap().is_same_bits(&tep));
        // Object(1) is ToObject(1): a Number wrapper, or (while wrapper.rs is the P0 stub)
        // the catchable "not implemented", never an object without the primitive.
        match cx.call(object, Local::undefined(), &[Local::int32(1)]) {
            Ok(w) => {
                let v = cx.this_primitive_value(w, PrimitiveType::Number).unwrap();
                assert_eq!(v.as_int32(), Some(1));
            }
            Err(_) => assert_eq!(
                take_error(cx, rt),
                "Error: not implemented in ep9 v0: wrapper objects for primitives"
            ),
        }
    });
}

// ---- table checks -----------------------------------------------------------------------------

fn check_one(m: BuiltinModule) -> String {
    let m: &'static [BuiltinModule] = Box::leak(Box::new([m]));
    let t: &'static BuiltinTable = Box::leak(Box::new(BuiltinTable::new("bad", m)));
    check_tables(&[crate::internals::VM_BUILTINS, t]).unwrap_err()
}

fn module(
    defs: &'static [BuiltinDef],
    intrinsics: &'static [IntrinsicDef],
    props: &'static [ObjectProps],
) -> BuiltinModule {
    BuiltinModule {
        name: "bad",
        block: 101,
        defs,
        intrinsics,
        props,
    }
}

fn on(target: IntrinsicId, props: &'static [PropDef]) -> &'static [ObjectProps] {
    Box::leak(Box::new([ObjectProps { target, props }]))
}

#[test]
fn table_checks_reject_each_broken_rule() {
    let e = check_one(BuiltinModule::empty("bad", 0));
    assert!(e.contains("block 0 is out of range or used twice"), "{e}");
    let e = check_one(BuiltinModule::empty("bad", 128));
    assert!(e.contains("block 128"), "{e}");
    let e = check_one(module(
        TEST_DEFS,
        const {
            &[IntrinsicDef {
                id: IntrinsicId::Object,
                value: IntrinsicValue::Builtin(1),
            }]
        },
        &[],
    ));
    assert!(
        e.contains("%Object% is a core intrinsic or made twice"),
        "{e}"
    );
    let e = check_one(module(
        TEST_DEFS,
        const {
            &[IntrinsicDef {
                id: IntrinsicId::Math,
                value: IntrinsicValue::Builtin(7),
            }]
        },
        &[],
    ));
    assert!(e.contains("definition 7 does not exist"), "{e}");
    // %Math% before the intrinsic that is its prototype.
    let e = check_one(module(
        TEST_DEFS,
        const {
            &[
                IntrinsicDef {
                    id: IntrinsicId::Math,
                    value: IntrinsicValue::Object {
                        proto: Some(IntrinsicId::Number),
                    },
                },
                IntrinsicDef {
                    id: IntrinsicId::Number,
                    value: IntrinsicValue::Builtin(1),
                },
            ]
        },
        &[],
    ));
    assert!(
        e.contains("%Math% has the prototype %Number%, which is not made before it"),
        "{e}"
    );
    let e = check_one(module(
        TEST_DEFS,
        &[],
        on(
            IntrinsicId::Math,
            const { &[PropDef::method(PropKey::Str("x"), 0)] },
        ),
    ));
    assert!(e.contains("target %Math% is not made"), "{e}");
    let e = check_one(module(
        TEST_DEFS,
        &[],
        on(
            IntrinsicId::ObjectPrototype,
            const { &[PropDef::method(PropKey::Str("toString"), 0)] },
        ),
    ));
    assert!(
        e.contains("%Object.prototype%.toString: defined twice"),
        "{e}"
    );
    let e = check_one(module(
        TEST_DEFS,
        &[],
        on(
            IntrinsicId::GlobalObject,
            const {
                &[PropDef::alias(
                    PropKey::Str("y"),
                    PropKey::Str("z"),
                    PropAttrs::NONE,
                )]
            },
        ),
    ));
    assert!(e.contains("alias of z, not defined before"), "{e}");
    let e = check_one(module(
        TEST_DEFS,
        &[],
        on(
            IntrinsicId::GlobalObject,
            const {
                &[PropDef::intrinsic(
                    PropKey::Str("N"),
                    IntrinsicId::Number,
                    PropAttrs::NONE,
                )]
            },
        ),
    ));
    assert!(e.contains("%Number% is not made"), "{e}");
    let e = check_one(module(
        TEST_DEFS,
        &[],
        on(
            IntrinsicId::GlobalObject,
            const { &[PropDef::accessor(PropKey::Str("g"), Some(0), None)] },
        ),
    ));
    assert!(e.contains("definition 0 is not a Getter"), "{e}");
    let e = check_one(module(
        TEST_DEFS,
        &[],
        on(
            IntrinsicId::GlobalObject,
            const {
                &[PropDef {
                    key: PropKey::Str("h"),
                    value: PropValue::Accessor {
                        get: None,
                        set: None,
                    },
                    attrs: PropAttrs::CONFIGURABLE,
                }]
            },
        ),
    ));
    assert!(
        e.contains("an accessor without a getter or a setter"),
        "{e}"
    );
    // A broken table never makes an isolate.
    static DUP: BuiltinTable = BuiltinTable::new(
        "dup",
        &[BuiltinModule::empty("a", 3), BuiltinModule::empty("b", 3)],
    );
    match Isolate::new(IsolateConfig {
        builtins: Some(&DUP),
        ..IsolateConfig::default()
    }) {
        Err(IsolateError::BuiltinTable(m)) => assert!(m.contains("block 3"), "{m}"),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("a duplicate block was accepted"),
    }
}

/// `%Number%` made by a table (block 104), and two of its keys.
const NUMBER_DEFS: &[BuiltinDef] = &[BuiltinDef::constructor("Number", 1, count_args)];
const MAKE_NUMBER: &[IntrinsicDef] = &[IntrinsicDef {
    id: IntrinsicId::Number,
    value: IntrinsicValue::Builtin(0),
}];
const NUMBER_PROTOTYPE: PropDef = PropDef::intrinsic(
    PropKey::Str("prototype"),
    IntrinsicId::NumberPrototype,
    PropAttrs::NONE,
);
const EPSILON: PropDef = PropDef::constant(
    PropKey::Str("EPSILON"),
    ConstValue::Number(f64::EPSILON),
    PropAttrs::NONE,
);

/// A table whose one module makes `%Number%` and gives it `props`.
fn number_table(props: &'static [PropDef]) -> &'static BuiltinTable {
    let m = module(NUMBER_DEFS, MAKE_NUMBER, on(IntrinsicId::Number, props));
    let m: &'static [BuiltinModule] = Box::leak(Box::new([m]));
    Box::leak(Box::new(BuiltinTable::new("number", m)))
}

/// The `IsolateError::BuiltinTable` text of an isolate with `table`.
fn table_error(table: &'static BuiltinTable) -> String {
    match Isolate::new(IsolateConfig {
        builtins: Some(table),
        ..IsolateConfig::default()
    }) {
        Err(IsolateError::BuiltinTable(m)) => m,
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("table `{}` was accepted", table.name),
    }
}

/// Install adds keys in table order, and a key added to a constructor first makes its lazy
/// `prototype` (`props::own_add`). `EPSILON` before `prototype` passed the checks, then
/// install met `prototype` twice: a panic in debug builds, a dictionary-mode `%Number%` in
/// release builds (P2 review). Now the checks want `prototype` {} first, and install rejects
/// a key the object has (a function's `name`) in every build.
#[test]
fn a_table_constructor_gets_prototype_first() {
    let e = table_error(number_table(const { &[EPSILON, NUMBER_PROTOTYPE] }));
    assert!(
        e.contains("%Number%.EPSILON: the first key of a constructor is `prototype`"),
        "{e}"
    );
    // `prototype` with other attributes, or no `prototype` at all.
    let e = check_one(module(
        NUMBER_DEFS,
        MAKE_NUMBER,
        on(
            IntrinsicId::Number,
            const {
                &[PropDef::intrinsic(
                    PropKey::Str("prototype"),
                    IntrinsicId::NumberPrototype,
                    PropAttrs::METHOD,
                )]
            },
        ),
    ));
    assert!(
        e.contains("%Number%.prototype: the first key of a constructor is `prototype`"),
        "{e}"
    );
    let e = check_one(module(NUMBER_DEFS, MAKE_NUMBER, &[]));
    assert!(
        e.contains("the constructor %Number% has no `prototype`"),
        "{e}"
    );
    // `name` comes from `new_function`, which the checks do not see.
    let name_again = number_table(
        const {
            &[
                NUMBER_PROTOTYPE,
                PropDef::constant(PropKey::Str("name"), ConstValue::Str("N"), PropAttrs::NONE),
            ]
        },
    );
    assert_eq!(
        check_tables(&[crate::internals::VM_BUILTINS, name_again]),
        Ok(())
    );
    let e = table_error(name_again);
    assert!(
        e.contains("%Number%.name: the object has this key already"),
        "{e}"
    );
    // The accepted order gives V8's: length, name, prototype {}, then the other keys, and
    // the constructor stays in fast mode.
    let number = number_table(const { &[NUMBER_PROTOTYPE, EPSILON] });
    for stress in [false, true] {
        with_table(Some(number), stress, |cx, rt| {
            let n = cx.intrinsic(IntrinsicId::Number);
            assert_eq!(keys(cx, rt, n), ["length", "name", "prototype", "EPSILON"]);
            let d = cx.get_own_property(n, "prototype").unwrap().unwrap();
            let np = cx.intrinsic(IntrinsicId::NumberPrototype);
            assert!(d.value.unwrap().is_same_bits(&np));
            assert_eq!(
                (d.writable, d.enumerable, d.configurable),
                (Some(false), Some(false), Some(false))
            );
            assert!(!crate::shape::is_dictionary(crate::shape::shape_of(
                n.bits()
            )));
        });
    }
}

fn get_answer<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::int32(7))
}

/// An accessor and a well-known symbol key: install needs accessors (P3-ACC) and symbols
/// (P2-SYM). Until they exist, `Isolate::new` fails with the reason, never a half-built
/// realm; once they do, the property works.
#[test]
fn accessor_and_symbol_entries_install_or_name_what_is_missing() {
    const DEFS: &[BuiltinDef] = &[
        BuiltinDef::getter("seven", get_answer),
        BuiltinDef::method("[Symbol.iterator]", 0, answer),
    ];
    static ACCESSOR: BuiltinTable = BuiltinTable::new(
        "accessor",
        &[BuiltinModule {
            name: "accessor",
            block: 102,
            defs: DEFS,
            intrinsics: &[],
            props: &[ObjectProps {
                target: IntrinsicId::GlobalObject,
                props: &[PropDef::accessor(PropKey::Str("seven"), Some(0), None)],
            }],
        }],
    );
    static SYMBOL_KEY: BuiltinTable = BuiltinTable::new(
        "symbol-key",
        &[BuiltinModule {
            name: "symbol-key",
            block: 103,
            defs: DEFS,
            intrinsics: &[],
            props: &[ObjectProps {
                target: IntrinsicId::ArrayPrototype,
                props: &[PropDef::method(
                    PropKey::Symbol(WellKnownSymbol::ITERATOR),
                    1,
                )],
            }],
        }],
    );
    match Isolate::new(IsolateConfig {
        builtins: Some(&ACCESSOR),
        ..IsolateConfig::default()
    }) {
        Ok(mut iso) => {
            let v = iso.with_cx(|cx| {
                let g = cx.global();
                cx.get(g, "seven").map(|v| v.as_int32())
            });
            assert_eq!(v, Ok(Some(7)));
        }
        Err(IsolateError::BuiltinTable(m)) => {
            assert!(
                m.contains("globalThis.seven") && m.contains("accessor"),
                "{m}"
            );
        }
        Err(e) => panic!("{e}"),
    }
    match Isolate::new(IsolateConfig {
        builtins: Some(&SYMBOL_KEY),
        ..IsolateConfig::default()
    }) {
        Ok(_) => {}
        Err(IsolateError::BuiltinTable(m)) => {
            assert!(m.contains("Symbol.iterator does not exist yet"), "{m}");
        }
        Err(e) => panic!("{e}"),
    }
}

// ---- the P3 surface -----------------------------------------------------------------------------

#[test]
fn function_name_and_length_helpers() {
    with_table(None, true, |cx, rt| {
        let f = global(cx, "Array").as_object().unwrap();
        let name = cx.new_string("x").unwrap();
        cx.set_function_name(f, name, Some("bound")).unwrap();
        cx.set_function_length(f, 3.0).unwrap();
        let d = cx.get_own_property(f, "name").unwrap().unwrap();
        assert_eq!(cx.to_rust_string(d.value.unwrap()).unwrap(), "bound x");
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(false), Some(false), Some(true))
        );
        let d = cx.get_own_property(f, "length").unwrap().unwrap();
        assert_eq!(d.value.unwrap().as_number(), Some(3.0));
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(false), Some(false), Some(true))
        );
        cx.set_function_length(f, f64::INFINITY).unwrap();
        let len = cx.get(f, "length").unwrap();
        assert_eq!(len.as_number(), Some(f64::INFINITY));
        // Not a property key.
        assert!(cx.set_function_name(f, Local::int32(1), None).is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: a function name is a string or a symbol"
        );
        // A non-configurable `name` cannot be redefined.
        let o = cx.new_object();
        let frozen = PropertyDescriptor::data(Local::int32(0), false, false, false);
        cx.define_property_or_throw(o, "name", frozen).unwrap();
        assert!(cx.set_function_name(o, name, None).is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: Cannot redefine property: name"
        );
    });
}

#[test]
fn this_primitive_value_and_wrappers() {
    with_table(None, true, |cx, rt| {
        let s = cx.new_string("ab").unwrap();
        for (v, ty) in [
            (Local::int32(3), PrimitiveType::Number),
            (Local::boolean(true), PrimitiveType::Boolean),
            (s.value(), PrimitiveType::String),
        ] {
            let r = cx.this_primitive_value(v, ty).unwrap();
            assert!(r.is_same_bits(&v));
        }
        let o = cx.new_object();
        assert!(cx.this_primitive_value(o, PrimitiveType::Number).is_err());
        assert!(
            take_error(cx, rt).starts_with(
                "TypeError: Number.prototype method called on an incompatible receiver"
            )
        );
        assert!(
            cx.this_primitive_value(Local::int32(1), PrimitiveType::String)
                .is_err()
        );
        rt.take_pending();
        // A wrapper of a non-primitive is a TypeError; ToObject(undefined) too.
        let np = cx.intrinsic(IntrinsicId::NumberPrototype);
        assert!(cx.new_wrapper(np, o).is_err());
        assert!(take_error(cx, rt).starts_with("TypeError: Cannot wrap"));
        assert!(cx.to_object(Local::undefined()).is_err());
        rt.take_pending();
        // ToObject of each primitive: its wrapper, or "not implemented" while wrapper.rs is
        // the stub. Never an object without the primitive.
        for (v, ty) in [
            (Local::number(1.5), PrimitiveType::Number),
            (Local::boolean(false), PrimitiveType::Boolean),
            (s.value(), PrimitiveType::String),
        ] {
            match cx.to_object(v) {
                Ok(w) => {
                    let p = cx.get_prototype_of(w).unwrap();
                    let want = match ty {
                        PrimitiveType::Number => IntrinsicId::NumberPrototype,
                        PrimitiveType::Boolean => IntrinsicId::BooleanPrototype,
                        _ => IntrinsicId::StringPrototype,
                    };
                    assert!(p.is_same_bits(&cx.intrinsic(want)));
                    let back = cx.this_primitive_value(w, ty).unwrap();
                    assert!(cx.same_value(back, v));
                }
                Err(_) => assert_eq!(
                    take_error(cx, rt),
                    "Error: not implemented in ep9 v0: wrapper objects for primitives"
                ),
            }
        }
    });
}

/// `Object.prototype.toString` with `this`.
fn to_string_tag<'s>(cx: &mut Cx<'s>, this: impl Into<Local<'s>>) -> String {
    let op = cx.intrinsic(IntrinsicId::ObjectPrototype);
    let f = cx.get(op, "toString").unwrap();
    let r = cx.call(f, this, &[]).unwrap();
    cx.to_rust_string(r).unwrap()
}

/// Object.prototype.toString takes the builtinTag of a wrapper from its [[BooleanData]],
/// [[NumberData]] or [[StringData]] (§20.1.3.6 steps 3 and 9-11), as node does:
/// `Object.prototype.toString.call(Object(7))` is "[object Number]" (P2 review).
#[test]
fn object_to_string_tags_a_wrapper_by_its_primitive() {
    with_table(None, true, |cx, _| {
        let s = cx.new_string("ab").unwrap();
        for (v, tag) in [
            (Local::int32(7), "[object Number]"),
            (Local::boolean(true), "[object Boolean]"),
            (s.value(), "[object String]"),
        ] {
            assert_eq!(to_string_tag(cx, v), tag);
            let w = cx.to_object(v).unwrap();
            assert_eq!(to_string_tag(cx, w), tag);
        }
        // The prototypes are wrappers of +0, "" and false (§21.1.3, §22.1.3, §20.3.3).
        for (id, tag) in [
            (IntrinsicId::NumberPrototype, "[object Number]"),
            (IntrinsicId::StringPrototype, "[object String]"),
            (IntrinsicId::BooleanPrototype, "[object Boolean]"),
        ] {
            let p = cx.intrinsic(id);
            assert_eq!(to_string_tag(cx, p), tag);
        }
        // A symbol's builtinTag is "Object" (step 14): node's "[object Symbol]" comes from
        // %Symbol.prototype%[@@toStringTag] (step 15, P3).
        let sym = cx.new_symbol(Local::undefined()).unwrap();
        assert_eq!(to_string_tag(cx, sym), "[object Object]");
        let w = cx.to_object(sym).unwrap();
        assert_eq!(to_string_tag(cx, w), "[object Object]");
        // Controls.
        let a = cx.new_array();
        assert_eq!(to_string_tag(cx, a), "[object Array]");
        let o = cx.new_object();
        assert_eq!(to_string_tag(cx, o), "[object Object]");
        let f = global(cx, "Object");
        assert_eq!(to_string_tag(cx, f), "[object Function]");
        assert_eq!(to_string_tag(cx, Local::undefined()), "[object Undefined]");
        assert_eq!(to_string_tag(cx, Local::null()), "[object Null]");
    });
}

/// `new_bound_function` forwards to `bound::new_bound_function` (P3-BLTA fills it) with the
/// arguments copied onto the VM stack.
fn bind_rest<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let target = this.as_function().unwrap();
    cx.new_bound_function(target, args.get(0), args.rest(1))
        .map(|f| f.value())
}

#[test]
fn new_bound_function_forwards_the_arguments() {
    with_table(None, true, |cx, rt| {
        let f = crate::realm::new_host_function(rt, bind_rest, "bindRest", 1, false);
        let f: Local<'_> = Local::from_bits(f);
        let target = global(cx, "Array");
        let r = cx.call(
            f,
            target,
            &[Local::int32(1), Local::int32(2), Local::int32(3)],
        );
        match r {
            Ok(b) => assert!(b.is_callable()),
            Err(_) => assert_eq!(
                take_error(cx, rt),
                "Error: not implemented in ep9 v0: bound functions"
            ),
        }
    });
}

#[test]
fn symbol_entry_points_wait_for_symbols() {
    with_table(None, false, |cx, rt| {
        let k = cx.new_string("k").unwrap();
        match cx.symbol_for(k) {
            Ok(s) => {
                let again = cx.symbol_for(k).unwrap();
                assert!(s.is_same_bits(&again));
                assert!(cx.key_for(s).is_some());
            }
            Err(_) => assert_eq!(
                take_error(cx, rt),
                "Error: not implemented in ep9 v0: symbols"
            ),
        }
        assert!(cx.new_symbol(Local::int32(1)).is_err());
        assert_eq!(
            take_error(cx, rt),
            "TypeError: a symbol description is a string or undefined"
        );
    });
}

// ---- GlobalDeclarationInstantiation (§16.1.7) in DeclareGlobals (P1 review) ------------------

/// The global gets `ro` = 1 {w: false, e: false, c: false} (`frozen`), then becomes
/// non-extensible (`prevent`).
fn setup(iso: &mut Isolate, frozen: bool, prevent: bool) {
    let mut cx = Cx::new(&mut iso.cx);
    let g = cx.global();
    if frozen {
        let d = PropertyDescriptor::data(Local::int32(1), false, false, false);
        cx.define_property_or_throw(g, "ro", d).unwrap();
    }
    if prevent {
        cx.prevent_extensions(g).unwrap();
    }
}

/// A script with the top-level declarations `decls`: DeclareGlobals, then for each
/// `Function` name a closure through InitializeGlobal.
fn script(decls: &'static [(&'static str, VariableMode)]) -> VerifiedUnit {
    let mut s = Asm::script(0);
    let init = s.b.new_register_list(2);
    s.b.call_runtime(RuntimeId::DeclareGlobals, RegisterList::EMPTY, 0);
    let mut fns = vec![];
    for &(name, mode) in decls {
        if mode == VariableMode::Function {
            let id = FunctionId(1 + fns.len() as u32);
            let mut f = Asm::new(meta(name, 0, 0, 0, 0));
            f.b.lda_smi(2).return_();
            fns.push(f.finish());
            s.lda_str(name);
            s.b.star(init.get(0));
            let k = s.b.function_constant(id);
            s.b.create_closure(k, s.closure, 0).star(init.get(1));
            s.b.call_runtime(RuntimeId::InitializeGlobal, init, 2);
        }
    }
    s.b.lda_undefined().return_();
    let mut all = vec![s.finish()];
    all.extend(fns);
    unit_with_scopes("", vec![(ScopeKind::Script, None, decls.to_vec())], all)
}

/// Runs `make` after `setup`, with and without GC stress. `error`: the expected constructor
/// name of the uncaught error, or `None` for success. Then `check`.
fn run(
    make: fn() -> VerifiedUnit,
    frozen: bool,
    prevent: bool,
    error: Option<&'static str>,
    check: fn(&mut Cx<'_>),
) {
    for stress in [false, true] {
        on_big_stack(move || {
            let mut iso = isolate(config(Dispatch::DEFAULT, stress));
            setup(&mut iso, frozen, prevent);
            match (iso.run_script(make(), "decl.js"), error) {
                (Ok(_), None) => {}
                (Err(e), Some(want)) => assert_eq!(
                    e.constructor.as_deref(),
                    Some(want),
                    "stress={stress}: {}",
                    e.message
                ),
                (Ok(_), Some(want)) => panic!("stress={stress}: expected a {want}"),
                (Err(e), None) => panic!("stress={stress}: {}", e.message),
            }
            {
                let mut cx = Cx::new(&mut iso.cx);
                check(&mut cx);
            }
            iso.collect_garbage();
            iso.verify_heap().unwrap();
        });
    }
}

fn absent(cx: &mut Cx<'_>, name: &str) {
    let g = cx.global();
    assert!(
        cx.get_own_property(g, name).unwrap().is_none(),
        "global {name} must not exist"
    );
}

/// test262 language/global-code/script-decl-var-err.js (CanDeclareGlobalVar, §9.1.1.4.15).
#[test]
fn a_new_global_var_needs_an_extensible_global() {
    run(
        || script(&[("brandNew", VariableMode::Var)]),
        false,
        true,
        Some("TypeError"),
        |cx| absent(cx, "brandNew"),
    );
}

/// test262 language/global-code/script-decl-func-err-non-extensible.js
/// (CanDeclareGlobalFunction, §9.1.1.4.16 step 4).
#[test]
fn a_new_global_function_needs_an_extensible_global() {
    run(
        || script(&[("brandNewFn", VariableMode::Function)]),
        false,
        true,
        Some("TypeError"),
        |cx| absent(cx, "brandNewFn"),
    );
}

/// test262 language/global-code/script-decl-func-err-non-configurable.js, and
/// `function NaN() {}`, which compiled JS reaches today (CanDeclareGlobalFunction step 6).
#[test]
fn a_global_function_cannot_replace_a_non_configurable_read_only_property() {
    fn check_ro(cx: &mut Cx<'_>) {
        let g = cx.global();
        let d = cx.get_own_property(g, "ro").unwrap().unwrap();
        assert_eq!(d.value.unwrap().as_int32(), Some(1));
        assert_eq!(
            (d.writable, d.enumerable, d.configurable),
            (Some(false), Some(false), Some(false))
        );
    }
    run(
        || script(&[("ro", VariableMode::Function)]),
        true,
        false,
        Some("TypeError"),
        check_ro,
    );
    run(
        || script(&[("NaN", VariableMode::Function)]),
        false,
        false,
        Some("TypeError"),
        |cx| {
            let g = cx.global();
            let d = cx.get_own_property(g, "NaN").unwrap().unwrap();
            assert!(d.value.unwrap().as_number().unwrap().is_nan());
        },
    );
}

/// §16.1.7 steps 3-10 check every name before step 16 creates any binding: `var zz; let NaN;`
/// throws a SyntaxError and leaves no `zz`.
#[test]
fn global_declaration_checks_run_before_any_binding_is_made() {
    run(
        || script(&[("zz", VariableMode::Var), ("NaN", VariableMode::Let)]),
        false,
        false,
        Some("SyntaxError"),
        |cx| absent(cx, "zz"),
    );
    // `var zz; function NaN() {}`: the function check (step 8) fails before any var exists.
    run(
        || script(&[("zz", VariableMode::Var), ("NaN", VariableMode::Function)]),
        false,
        false,
        Some("TypeError"),
        |cx| absent(cx, "zz"),
    );
}

/// Controls that must keep working: `var ro` over a frozen property, and lexical
/// declarations on a non-extensible global (they live in the lexical record).
#[test]
fn allowed_global_declarations_on_a_non_extensible_global() {
    run(
        || script(&[("ro", VariableMode::Var), ("lex", VariableMode::Let)]),
        true,
        true,
        None,
        |cx| {
            let g = cx.global();
            let ro = cx.get(g, "ro").unwrap();
            assert_eq!(ro.as_int32(), Some(1));
            absent(cx, "lex");
        },
    );
}

/// Bindings are made after the checks: functions, then vars (§16.1.7 steps 16-17), and a
/// configurable or writable-enumerable global can take a function declaration.
#[test]
fn global_bindings_functions_then_vars() {
    run(
        || {
            script(&[
                ("v1", VariableMode::Var),
                ("f1", VariableMode::Function),
                ("Object", VariableMode::Function),
                ("c1", VariableMode::Const),
            ])
        },
        false,
        false,
        None,
        |cx| {
            let g = cx.global();
            let rt = cx.inner().rt();
            let ks = keys(cx, rt, g);
            let pos = |k: &str| ks.iter().position(|x| x == k).unwrap();
            assert!(pos("f1") < pos("v1"), "{ks:?}");
            absent(cx, "c1");
            let f1 = cx.get(g, "f1").unwrap();
            assert!(f1.is_callable());
            let object = cx.get(g, "Object").unwrap();
            assert!(object.is_callable());
        },
    );
}
