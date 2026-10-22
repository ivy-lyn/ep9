//! Feedback slot kinds, runtime function ids and `typeof` literals.
//!
//! [`FeedbackSlotKind`] is the **H16 v0** slot-kind list (`docs/adr/H16-ics-and-feedback.md`).
//! It is also the FactSnapshot v0 schema (`ep9_facts`), and it freezes at M1 exit. It follows
//! V8's `FeedbackSlotKind` without the language-mode split: the VM reads strictness from the
//! function flags. The byte values and names are part of the bytecode ABI (the opcode table
//! prints `Slot(<name>)`), so a change bumps [`crate::ABI_VERSION`] and
//! `ep9_facts::FACT_SCHEMA_VERSION` (`docs/adr/H02-bytecode-abi.md`, change log).
//!
//! Each slot has three views, listed on each kind:
//! - **Opcodes**: the instructions whose `Slot(<kind>)` operand names the slot.
//! - **Runtime record**: one 16-byte entry `[key | value]` in the function's FeedbackVector.
//!   The key word is weak and the value word is strong (H16). A new entry is
//!   `[undefined | int32 0]`. T0 v0 writes only the property-IC entries of opcodes 0x21,
//!   0x22, 0x24 and 0x25; the other records are defined now and written from M1 (H16-Q2).
//! - **Fact record**: the `ep9_facts` payload that a FactSnapshot stores for the slot. It
//!   holds no ShapeId and no address.
//!
//! [`RuntimeId`] stays a **placeholder** with 19 ids (0-18) until the H13 index lists (M1).

macro_rules! byte_enum {
    (
        $(#[$doc:meta])*
        pub enum $name:ident { $( $(#[$vdoc:meta])* $variant:ident = $value:literal => $text:literal, )* }
    ) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u8)]
        pub enum $name { $( $(#[$vdoc])* $variant = $value, )* }

        impl $name {
            /// Every value, in byte order.
            pub const ALL: &'static [$name] = &[ $( $name::$variant, )* ];

            /// The value with byte `b`, if it is defined.
            pub const fn from_u8(b: u8) -> Option<$name> {
                match b {
                    $( $value => Some($name::$variant), )*
                    _ => None,
                }
            }

            /// Name used in the disassembly and golden files.
            pub const fn name(self) -> &'static str {
                match self { $( $name::$variant => $text, )* }
            }
        }

        // Values are dense: ALL[i] has byte value i.
        const _: () = {
            let all = $name::ALL;
            let mut i = 0;
            while i < all.len() {
                assert!(all[i] as usize == i, "values must be 0, 1, 2, ... in order");
                i += 1;
            }
        };
    };
}

byte_enum! {
    /// The kind of one feedback slot (H16 v0). Each opcode operand of kind
    /// [`crate::OperandKind::Slot`] names the kind its slot must have; the verifier checks it.
    ///
    /// "Property IC" below is the H16 IC entry: key `undefined` = uninitialized, a shape cell
    /// = monomorphic, the hole with a POLY_IC cell in the value = polymorphic (at most 4
    /// cases), the hole with an int32 in the value = megamorphic (sticky).
    pub enum FeedbackSlotKind {
        /// Call and construct sites.
        ///
        /// - Opcodes: 0x5e-0x67 (`CallAnyReceiver` to `CallWithSpread`), 0x69 `Construct`, 0x6a
        ///   `ConstructWithSpread`, 0x6b `TailCall`, and the `call_slot` of 0x8f `GetIterator`.
        ///   0x68 `CallRuntime` has no slot.
        /// - Runtime record: key = the callee (weak) while monomorphic, the hole once megamorphic;
        ///   value = int32 call count.
        /// - Fact record: `CallFacts`: call count, IC state, up to 4 targets (`FunctionRef`),
        ///   result `TypeSet`.
        Call = 0 => "Call",
        /// Named property load.
        ///
        /// - Opcodes: 0x21 `GetNamedProperty`, 0x22 `GetNamedPropertyFromSuper` (keyed on the
        ///   lookup-start object), and the `load_slot` of 0x8f `GetIterator`.
        /// - Runtime record: property IC.
        /// - Fact record: `PropertyFacts`: IC state, up to 4 cases (receiver `ShapeDesc`,
        ///   `Access`), value `TypeSet`.
        LoadNamed = 1 => "LoadNamed",
        /// Keyed property load.
        ///
        /// - Opcodes: 0x23 `GetKeyedProperty`.
        /// - Runtime record: property IC.
        /// - Fact record: `KeyedFacts`: `PropertyFacts` plus the `KeySet` of keys seen.
        LoadKeyed = 2 => "LoadKeyed",
        /// Named property store.
        ///
        /// - Opcodes: 0x24 `SetNamedProperty`.
        /// - Runtime record: property IC (an add-property case names the target shape, weak).
        /// - Fact record: `PropertyFacts`; the value `TypeSet` holds the stored values.
        StoreNamed = 3 => "StoreNamed",
        /// Keyed property store.
        ///
        /// - Opcodes: 0x26 `SetKeyedProperty`.
        /// - Runtime record: property IC.
        /// - Fact record: `KeyedFacts`.
        StoreKeyed = 4 => "StoreKeyed",
        /// Own named property definition.
        ///
        /// - Opcodes: 0x25 `DefineNamedOwnProperty`.
        /// - Runtime record: property IC.
        /// - Fact record: `PropertyFacts`.
        DefineNamedOwn = 5 => "DefineNamedOwn",
        /// Own keyed property definition.
        ///
        /// - Opcodes: 0x27 `DefineKeyedOwnProperty`, 0x29 `DefineKeyedOwnPropertyInLiteral`.
        /// - Runtime record: property IC.
        /// - Fact record: `KeyedFacts`.
        DefineKeyedOwn = 6 => "DefineKeyedOwn",
        /// Global load outside `typeof`.
        ///
        /// - Opcodes: 0x10 `LdaGlobal`.
        /// - Runtime record: key = the shape of the object that held the binding (weak); value =
        ///   the handler.
        /// - Fact record: `GlobalFacts`: the `BindingSet` (lexical, data property, accessor,
        ///   missing) and the value `TypeSet`.
        LoadGlobal = 7 => "LoadGlobal",
        /// Global load inside `typeof` (no ReferenceError).
        ///
        /// - Opcodes: 0x11 `LdaGlobalInsideTypeof`.
        /// - Runtime record: as `LoadGlobal`.
        /// - Fact record: `GlobalFacts`.
        LoadGlobalInsideTypeof = 8 => "LoadGlobalInsideTypeof",
        /// Global store.
        ///
        /// - Opcodes: 0x12 `StaGlobal`.
        /// - Runtime record: as `LoadGlobal`.
        /// - Fact record: `GlobalFacts`; the value `TypeSet` holds the stored values.
        StoreGlobal = 9 => "StoreGlobal",
        /// `in` operator.
        ///
        /// - Opcodes: 0x53 `TestIn`.
        /// - Runtime record: property IC.
        /// - Fact record: `KeyedFacts`; a case is an own or prototype hit (`Access::OwnField`,
        ///   `Access::ProtoField`) or `Access::Missing`.
        HasKeyed = 10 => "HasKeyed",
        /// Binary and unary arithmetic, bitwise ops, `ToNumber`, `ToNumeric`.
        ///
        /// - Opcodes: 0x2a-0x45 (`Add` to `BitwiseNot`, the `*Smi` forms, `Inc`, `Dec`, `Negate`),
        ///   0x59 `ToNumber`, 0x5a `ToNumeric`.
        /// - Runtime record: key unused; value = int32 `BinaryHint` bits.
        /// - Fact record: `BinaryHint`.
        BinaryOp = 11 => "BinaryOp",
        /// Comparisons.
        ///
        /// - Opcodes: 0x4b-0x50 (`TestEqual` to `TestGreaterThanOrEqual`).
        /// - Runtime record: key unused; value = int32 `CompareHint` bits.
        /// - Fact record: `CompareHint`.
        CompareOp = 12 => "CompareOp",
        /// `typeof`.
        ///
        /// - Opcodes: 0x48 `TypeOf`.
        /// - Runtime record: key unused; value = int32 `TypeSet` bits.
        /// - Fact record: `TypeSet`.
        TypeOf = 13 => "TypeOf",
        /// `instanceof`.
        ///
        /// - Opcodes: 0x52 `TestInstanceOf`.
        /// - Runtime record: key = the right-hand side constructor (weak); value = int32 IC state.
        /// - Fact record: `TargetFacts`: IC state and up to 4 constructors (`FunctionRef`).
        InstanceOf = 14 => "InstanceOf",
        /// Literal sites: object, array, RegExp and template objects (allocation site).
        ///
        /// - Opcodes: 0x6c `CreateRegExpLiteral`, 0x6d `CreateArrayLiteral`, 0x6e
        ///   `CreateEmptyArrayLiteral`, 0x70 `CreateObjectLiteral`, 0x73 `GetTemplateObject`.
        /// - Runtime record: key = the shape of the last literal (weak); value = int32 allocation
        ///   count.
        /// - Fact record: `LiteralFacts`: allocation count, result `ShapeDesc`, H18 `ElementsKind`.
        Literal = 15 => "Literal",
        /// `CloneObject` (object spread).
        ///
        /// - Opcodes: 0x72 `CloneObject`.
        /// - Runtime record: key = the source shape (weak); value = a handler that names the result
        ///   shape (weak).
        /// - Fact record: `CloneFacts`: IC state and up to 4 (source, result) `ShapeDesc` pairs.
        CloneObject = 16 => "CloneObject",
        /// Stores into array literals.
        ///
        /// - Opcodes: 0x28 `StaInArrayLiteral`.
        /// - Runtime record: property IC.
        /// - Fact record: `KeyedFacts` (cases are `Access::Element`).
        StoreInArrayLiteral = 17 => "StoreInArrayLiteral",
        /// `for-in` enumeration cache.
        ///
        /// - Opcodes: 0x8c `ForInPrepare`, 0x8d `ForInNext`.
        /// - Runtime record: key unused; value = int32 `ForInHint` bits.
        /// - Fact record: `ForInHint`.
        ForIn = 18 => "ForIn",
        /// Loop back edge: OSR and back-edge counters.
        ///
        /// - Opcodes: 0x7d `JumpLoop`.
        /// - Runtime record: key unused; value = int32 back-edge count.
        /// - Fact record: `LoopFacts`: back-edge count.
        JumpLoop = 19 => "JumpLoop",
        /// Closure creation: the feedback cell of the new closure.
        ///
        /// - Opcodes: 0x74 `CreateClosure`.
        /// - Runtime record: key = the one closure made here (weak); value = int32 0, 1 or 2 (none,
        ///   one, many).
        /// - Fact record: `ClosureFacts`: none, one or many closures.
        CreateClosure = 20 => "CreateClosure",
    }
}

byte_enum! {
    /// The `typeof` result that `TestTypeOf` compares against (V8 `TestTypeOfFlags`).
    pub enum TypeOfLiteral {
        /// `"number"`
        Number = 0 => "number",
        /// `"string"`
        String = 1 => "string",
        /// `"symbol"`
        Symbol = 2 => "symbol",
        /// `"boolean"`
        Boolean = 3 => "boolean",
        /// `"bigint"`
        BigInt = 4 => "bigint",
        /// `"undefined"`
        Undefined = 5 => "undefined",
        /// `"function"`
        Function = 6 => "function",
        /// `"object"`
        Object = 7 => "object",
        /// Any other string: always false.
        Other = 8 => "other",
    }
}

/// Runtime function id: the [`crate::OperandKind::Runtime`] operand of `CallRuntime`.
///
/// **Placeholder list.** The ids are stable ABI values (H13: stable indices). BytecodeGen and
/// the VM add entries as they need them; each addition bumps [`crate::ABI_VERSION`] until H2
/// freezes at M1 exit. ABI 2 added ids 6-18 for BytecodeGen v0 (`ep9_frontend::bytecodegen`)
/// and fixed the `DefineClass` layout. ABI 3 added ids 19-28 (generators, class fields and
/// private names, `with` calls, sloppy direct eval). ABI 4 added ids 29-31 (a dynamic
/// reference resolved before the right-hand side). [`runtime_table_text`] is part of the ABI
/// fingerprint.
///
/// A function that returns a pair ([`RuntimeInfo::returns_pair`]) is called only by
/// `CallRuntimeForPair`, and every other function only by `CallRuntime` (the verifier checks
/// both).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum RuntimeId {
    /// `ThrowConstAssignError()`: assignment to a `const` binding.
    ThrowConstAssignError = 0,
    /// `ThrowIteratorResultNotAnObject(result)`.
    ThrowIteratorResultNotAnObject = 1,
    /// `ThrowSymbolIteratorInvalid()`.
    ThrowSymbolIteratorInvalid = 2,
    /// `CreateIterResultObject(value, done)`.
    CreateIterResultObject = 3,
    /// `DefineClass(constructor, heritage)`: ClassDefinitionEvaluation steps for the prototype
    /// and the constructor parent (`heritage` is the hole when the class has no `extends`).
    /// Sets `constructor.prototype`, `prototype.constructor` and the constructor's home object.
    /// Returns the prototype. (ABI 2 fixes the layout; ABI 1 had it variadic.)
    DefineClass = 4,
    /// `DeclareGlobals()`: GlobalDeclarationInstantiation for the running script.
    DeclareGlobals = 5,
    /// `InitializeGlobal(name, value)`: initializes a global binding that `DeclareGlobals`
    /// created: a lexical one (`let`, `const`, `class`) or a function declaration (defines the
    /// global object property, CreateGlobalFunctionBinding).
    InitializeGlobal = 6,
    /// `ResolvePossiblyDirectEval(callee, source, function, scope, position)`: if `callee` is
    /// %eval% and `source` is a string, compiles it as direct eval code for scope `scope` (a
    /// unit scope id as a Smi) at source byte `position` of `function`, and returns the new
    /// closure; else returns `callee`. The result is then called.
    ResolvePossiblyDirectEval = 7,
    /// `DeleteLookupSlot(name)`: sloppy `delete name` for a name that is not statically bound.
    DeleteLookupSlot = 8,
    /// `CopyDataProperties(target, source)`: object literal spread after the first property.
    CopyDataProperties = 9,
    /// `CopyDataPropertiesExcluding(source, excluded...)`: object rest in destructuring.
    /// Variadic. Returns a new object.
    CopyDataPropertiesExcluding = 10,
    /// `SetPrototypeInLiteral(object, proto)`: `__proto__: proto` in an object literal (sets the
    /// prototype only if `proto` is an object or null).
    SetPrototypeInLiteral = 11,
    /// `DefineAccessorInLiteral(object, key, getter, setter)`: an enumerable accessor of an
    /// object literal. `getter` or `setter` is undefined when absent. Names an anonymous
    /// function from `key`.
    DefineAccessorInLiteral = 12,
    /// `SetHomeObject(function, home)`: `[[HomeObject]]` of an object literal method that uses
    /// `super`.
    SetHomeObject = 13,
    /// `GetHomeObject(function)`: `[[HomeObject]]` of `function` (`super.x` in a method).
    GetHomeObject = 14,
    /// `DefineClassMethod(home, key, function)`: a non-enumerable class method on the prototype
    /// or the constructor; sets the home object and names an anonymous function from `key`.
    DefineClassMethod = 15,
    /// `DefineClassAccessor(home, key, getter, setter)`: a non-enumerable class accessor;
    /// `getter` or `setter` is undefined when absent.
    DefineClassAccessor = 16,
    /// `LoadKeyedFromSuper(receiver, home, key)`: `super[key]`.
    LoadKeyedFromSuper = 17,
    /// `StoreToSuper(receiver, home, key, value)`: `super[key] = value` and `super.x = value`.
    StoreToSuper = 18,
    /// `CreateGeneratorObject(function, receiver)`: the generator object of the running
    /// generator (or async) function `function`, with `receiver` as its `this`. The VM takes the
    /// current context and the register count from the frame (V8 `CreateJSGeneratorObject`).
    CreateGeneratorObject = 19,
    /// `GeneratorGetResumeMode(generator)`: how the generator was resumed (next, return or
    /// throw), as a Smi.
    GeneratorGetResumeMode = 20,
    /// `GeneratorClose(generator)`: marks the generator as completed.
    GeneratorClose = 21,
    /// `ThrowThrowMethodMissing()`: the TypeError of `yield*` when the inner iterator has no
    /// `throw` method (ECMA-262 §15.5.5 step 7.b.iii).
    ThrowThrowMethodMissing = 22,
    /// `CreatePrivateName(description)`: a new private name symbol (`#x`, or a class brand).
    CreatePrivateName = 23,
    /// `SetClassFieldsInitializer(constructor, initializer)`: stores the instance members
    /// initializer function on the class constructor, under a VM-internal private symbol.
    SetClassFieldsInitializer = 24,
    /// `RunClassFieldsInitializer(constructor, receiver)`: calls the initializer that
    /// `SetClassFieldsInitializer` stored on `constructor`, with `receiver` as `this` (at base
    /// constructor entry, and after `super()` returns). Does nothing if there is none.
    RunClassFieldsInitializer = 25,
    /// `LoadLookupSlotForCall(name, strict)`: dynamic lookup of `name` for a call. Returns a
    /// pair (function, receiver): the receiver is the `with` object that holds `name`, else
    /// undefined (ECMA-262 §9.1.1.2.10 WithBaseObject). `strict` is true or false: the call is
    /// in strict code (GetValue of a `with` binding that is gone throws there; ABI 4). Called
    /// with `CallRuntimeForPair`.
    LoadLookupSlotForCall = 26,
    /// `DeclareEvalVar(name)`: a `var` of sloppy direct eval code (EvalDeclarationInstantiation).
    DeclareEvalVar = 27,
    /// `DeclareEvalFunction(name, function)`: a function declaration of sloppy direct eval
    /// code.
    DeclareEvalFunction = 28,
    /// `ResolveLookupSlot(name)`: ResolveBinding (ECMA-262 §9.4.2) of a dynamic name, without
    /// GetValue, for a reference that is used after other code runs (`x = v`, `x op= v`,
    /// `x++`, `var x = v`). Returns the environment that binds `name`: a context, an object
    /// (a `with` object), undefined (the global environment) or null (unresolvable). Only
    /// `LoadLookupSlotResolved` and `StoreLookupSlotResolved` read the result. (ABI 4.)
    ResolveLookupSlot = 29,
    /// `LoadLookupSlotResolved(env, name, strict)`: GetValue of the reference that
    /// `ResolveLookupSlot` gave `env` for `name` (no new resolution); `strict` is true or false:
    /// the read is in strict code. (ABI 4.)
    LoadLookupSlotResolved = 30,
    /// `StoreLookupSlotResolved(env, name, value, strict)`: PutValue of `value` to that
    /// reference (SetMutableBinding of the record it names; `strict` is true or false: the
    /// store is in strict code). Returns `value`. (ABI 4.)
    StoreLookupSlotResolved = 31,
}

/// Static facts about one runtime function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeInfo {
    /// The id.
    pub id: RuntimeId,
    /// Name in the disassembly.
    pub name: &'static str,
    /// Exact argument count, or `None` for a variadic function.
    pub arity: Option<u32>,
    /// The function returns two values: only `CallRuntimeForPair` calls it (ABI 3).
    pub returns_pair: bool,
}

/// Every runtime function, in id order.
pub const RUNTIME_FUNCTIONS: &[RuntimeInfo] = &[
    RuntimeInfo {
        id: RuntimeId::ThrowConstAssignError,
        name: "ThrowConstAssignError",
        arity: Some(0),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::ThrowIteratorResultNotAnObject,
        name: "ThrowIteratorResultNotAnObject",
        arity: Some(1),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::ThrowSymbolIteratorInvalid,
        name: "ThrowSymbolIteratorInvalid",
        arity: Some(0),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::CreateIterResultObject,
        name: "CreateIterResultObject",
        arity: Some(2),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::DefineClass,
        name: "DefineClass",
        arity: Some(2),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::DeclareGlobals,
        name: "DeclareGlobals",
        arity: Some(0),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::InitializeGlobal,
        name: "InitializeGlobal",
        arity: Some(2),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::ResolvePossiblyDirectEval,
        name: "ResolvePossiblyDirectEval",
        arity: Some(5),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::DeleteLookupSlot,
        name: "DeleteLookupSlot",
        arity: Some(1),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::CopyDataProperties,
        name: "CopyDataProperties",
        arity: Some(2),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::CopyDataPropertiesExcluding,
        name: "CopyDataPropertiesExcluding",
        arity: None,
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::SetPrototypeInLiteral,
        name: "SetPrototypeInLiteral",
        arity: Some(2),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::DefineAccessorInLiteral,
        name: "DefineAccessorInLiteral",
        arity: Some(4),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::SetHomeObject,
        name: "SetHomeObject",
        arity: Some(2),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::GetHomeObject,
        name: "GetHomeObject",
        arity: Some(1),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::DefineClassMethod,
        name: "DefineClassMethod",
        arity: Some(3),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::DefineClassAccessor,
        name: "DefineClassAccessor",
        arity: Some(4),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::LoadKeyedFromSuper,
        name: "LoadKeyedFromSuper",
        arity: Some(3),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::StoreToSuper,
        name: "StoreToSuper",
        arity: Some(4),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::CreateGeneratorObject,
        name: "CreateGeneratorObject",
        arity: Some(2),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::GeneratorGetResumeMode,
        name: "GeneratorGetResumeMode",
        arity: Some(1),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::GeneratorClose,
        name: "GeneratorClose",
        arity: Some(1),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::ThrowThrowMethodMissing,
        name: "ThrowThrowMethodMissing",
        arity: Some(0),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::CreatePrivateName,
        name: "CreatePrivateName",
        arity: Some(1),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::SetClassFieldsInitializer,
        name: "SetClassFieldsInitializer",
        arity: Some(2),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::RunClassFieldsInitializer,
        name: "RunClassFieldsInitializer",
        arity: Some(2),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::LoadLookupSlotForCall,
        name: "LoadLookupSlotForCall",
        arity: Some(2),
        returns_pair: true,
    },
    RuntimeInfo {
        id: RuntimeId::DeclareEvalVar,
        name: "DeclareEvalVar",
        arity: Some(1),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::DeclareEvalFunction,
        name: "DeclareEvalFunction",
        arity: Some(2),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::ResolveLookupSlot,
        name: "ResolveLookupSlot",
        arity: Some(1),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::LoadLookupSlotResolved,
        name: "LoadLookupSlotResolved",
        arity: Some(3),
        returns_pair: false,
    },
    RuntimeInfo {
        id: RuntimeId::StoreLookupSlotResolved,
        name: "StoreLookupSlotResolved",
        arity: Some(4),
        returns_pair: false,
    },
];

const _: () = {
    let mut i = 0;
    while i < RUNTIME_FUNCTIONS.len() {
        assert!(
            RUNTIME_FUNCTIONS[i].id as usize == i,
            "runtime ids must be dense and in order"
        );
        i += 1;
    }
};

impl RuntimeId {
    /// The id with value `v`, if it is defined.
    pub const fn from_u16(v: u16) -> Option<RuntimeId> {
        if (v as usize) < RUNTIME_FUNCTIONS.len() {
            Some(RUNTIME_FUNCTIONS[v as usize].id)
        } else {
            None
        }
    }

    /// Static facts about this function.
    pub const fn info(self) -> &'static RuntimeInfo {
        &RUNTIME_FUNCTIONS[self as usize]
    }
}

/// The runtime function table as stable text: one line per id (`<id> <name> <arity>`, then
/// ` pair` for a function that returns a pair). The ABI guard (`tests/abi.rs`) hashes it, so a
/// change to the table needs an ABI bump.
pub fn runtime_table_text() -> String {
    let mut s = String::new();
    for info in RUNTIME_FUNCTIONS {
        s.push_str(&(info.id as u16).to_string());
        s.push(' ');
        s.push_str(info.name);
        s.push(' ');
        match info.arity {
            Some(n) => s.push_str(&n.to_string()),
            None => s.push_str("variadic"),
        }
        if info.returns_pair {
            s.push_str(" pair");
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_values_round_trip() {
        for &k in FeedbackSlotKind::ALL {
            assert_eq!(FeedbackSlotKind::from_u8(k as u8), Some(k));
        }
        assert_eq!(
            FeedbackSlotKind::from_u8(FeedbackSlotKind::ALL.len() as u8),
            None
        );
        for &t in TypeOfLiteral::ALL {
            assert_eq!(TypeOfLiteral::from_u8(t as u8), Some(t));
        }
        assert_eq!(TypeOfLiteral::from_u8(9), None);
        for info in RUNTIME_FUNCTIONS {
            assert_eq!(RuntimeId::from_u16(info.id as u16), Some(info.id));
            assert_eq!(info.id.info().name, info.name);
        }
        assert_eq!(RuntimeId::from_u16(RUNTIME_FUNCTIONS.len() as u16), None);
    }
}
