//! Feedback slot kinds, runtime function ids and `typeof` literals.
//!
//! [`FeedbackSlotKind`] is a **placeholder** for the H16 slot kinds (due M0 week 4, frozen at
//! M1 exit as the FactSnapshot v0 schema). It follows V8's `FeedbackSlotKind` without the
//! language-mode split: the VM reads strictness from the function flags. H16 may replace it;
//! the byte values are part of the bytecode ABI, so a change bumps [`crate::ABI_VERSION`].

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
    /// The kind of one feedback slot (H16 placeholder). Each opcode operand of kind
    /// [`crate::OperandKind::Slot`] names the kind its slot must have; the verifier checks it.
    pub enum FeedbackSlotKind {
        /// Call and construct sites (`Call*`, `Construct*`, the call part of `GetIterator`).
        Call = 0 => "Call",
        /// Named property load (`GetNamedProperty*`, the load part of `GetIterator`).
        LoadNamed = 1 => "LoadNamed",
        /// Keyed property load.
        LoadKeyed = 2 => "LoadKeyed",
        /// Named property store (`SetNamedProperty`).
        StoreNamed = 3 => "StoreNamed",
        /// Keyed property store (`SetKeyedProperty`).
        StoreKeyed = 4 => "StoreKeyed",
        /// Own named property definition.
        DefineNamedOwn = 5 => "DefineNamedOwn",
        /// Own keyed property definition.
        DefineKeyedOwn = 6 => "DefineKeyedOwn",
        /// Global load outside `typeof`.
        LoadGlobal = 7 => "LoadGlobal",
        /// Global load inside `typeof` (no ReferenceError).
        LoadGlobalInsideTypeof = 8 => "LoadGlobalInsideTypeof",
        /// Global store.
        StoreGlobal = 9 => "StoreGlobal",
        /// `in` operator.
        HasKeyed = 10 => "HasKeyed",
        /// Binary and unary arithmetic, bitwise ops, `ToNumber`, `ToNumeric`.
        BinaryOp = 11 => "BinaryOp",
        /// Comparisons.
        CompareOp = 12 => "CompareOp",
        /// `typeof`.
        TypeOf = 13 => "TypeOf",
        /// `instanceof`.
        InstanceOf = 14 => "InstanceOf",
        /// Literal sites: object, array, RegExp and template objects (allocation site).
        Literal = 15 => "Literal",
        /// `CloneObject` (object spread).
        CloneObject = 16 => "CloneObject",
        /// Stores into array literals.
        StoreInArrayLiteral = 17 => "StoreInArrayLiteral",
        /// `for-in` enumeration cache.
        ForIn = 18 => "ForIn",
        /// Loop back edge: OSR and back-edge counters.
        JumpLoop = 19 => "JumpLoop",
        /// Closure creation: the feedback cell of the new closure.
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
/// and fixed the `DefineClass` layout. [`runtime_table_text`] is part of the ABI fingerprint.
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
}

/// Every runtime function, in id order.
pub const RUNTIME_FUNCTIONS: &[RuntimeInfo] = &[
    RuntimeInfo {
        id: RuntimeId::ThrowConstAssignError,
        name: "ThrowConstAssignError",
        arity: Some(0),
    },
    RuntimeInfo {
        id: RuntimeId::ThrowIteratorResultNotAnObject,
        name: "ThrowIteratorResultNotAnObject",
        arity: Some(1),
    },
    RuntimeInfo {
        id: RuntimeId::ThrowSymbolIteratorInvalid,
        name: "ThrowSymbolIteratorInvalid",
        arity: Some(0),
    },
    RuntimeInfo {
        id: RuntimeId::CreateIterResultObject,
        name: "CreateIterResultObject",
        arity: Some(2),
    },
    RuntimeInfo {
        id: RuntimeId::DefineClass,
        name: "DefineClass",
        arity: Some(2),
    },
    RuntimeInfo {
        id: RuntimeId::DeclareGlobals,
        name: "DeclareGlobals",
        arity: Some(0),
    },
    RuntimeInfo {
        id: RuntimeId::InitializeGlobal,
        name: "InitializeGlobal",
        arity: Some(2),
    },
    RuntimeInfo {
        id: RuntimeId::ResolvePossiblyDirectEval,
        name: "ResolvePossiblyDirectEval",
        arity: Some(5),
    },
    RuntimeInfo {
        id: RuntimeId::DeleteLookupSlot,
        name: "DeleteLookupSlot",
        arity: Some(1),
    },
    RuntimeInfo {
        id: RuntimeId::CopyDataProperties,
        name: "CopyDataProperties",
        arity: Some(2),
    },
    RuntimeInfo {
        id: RuntimeId::CopyDataPropertiesExcluding,
        name: "CopyDataPropertiesExcluding",
        arity: None,
    },
    RuntimeInfo {
        id: RuntimeId::SetPrototypeInLiteral,
        name: "SetPrototypeInLiteral",
        arity: Some(2),
    },
    RuntimeInfo {
        id: RuntimeId::DefineAccessorInLiteral,
        name: "DefineAccessorInLiteral",
        arity: Some(4),
    },
    RuntimeInfo {
        id: RuntimeId::SetHomeObject,
        name: "SetHomeObject",
        arity: Some(2),
    },
    RuntimeInfo {
        id: RuntimeId::GetHomeObject,
        name: "GetHomeObject",
        arity: Some(1),
    },
    RuntimeInfo {
        id: RuntimeId::DefineClassMethod,
        name: "DefineClassMethod",
        arity: Some(3),
    },
    RuntimeInfo {
        id: RuntimeId::DefineClassAccessor,
        name: "DefineClassAccessor",
        arity: Some(4),
    },
    RuntimeInfo {
        id: RuntimeId::LoadKeyedFromSuper,
        name: "LoadKeyedFromSuper",
        arity: Some(3),
    },
    RuntimeInfo {
        id: RuntimeId::StoreToSuper,
        name: "StoreToSuper",
        arity: Some(4),
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

/// The runtime function table as stable text: one line per id (`<id> <name> <arity>`). The
/// ABI guard (`tests/abi.rs`) hashes it, so a change to the table needs an ABI bump.
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
