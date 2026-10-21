//! Byte enums and bitsets of the fact schema. Every value has a fixed byte or bit; the schema
//! text (`schema_text`) lists them all, so a change here changes the schema fingerprint.
//!
//! Bitsets only grow: two facts about one slot merge by OR (`|`), whatever their source. A
//! reserved bit is never set; the decoder rejects it.

/// A `u8` enum with dense values 0, 1, 2, ...: `ALL`, `from_u8` and `name`.
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

            /// Name in the text dump and the schema text.
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

/// A bitset over an unsigned integer with dense bits 0, 1, 2, ...; the bits above are
/// reserved.
macro_rules! bitset {
    (
        $(#[$doc:meta])*
        pub struct $name:ident($repr:ty) { $( $(#[$bdoc:meta])* $flag:ident = $bit:literal => $text:literal, )* }
    ) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($repr);

        impl $name {
            $( $(#[$bdoc])* pub const $flag: $name = $name(1 << $bit); )*

            /// No bit set.
            pub const EMPTY: $name = $name(0);
            /// Every defined bit.
            pub const ALL: $name = $name(0 $( | (1 << $bit) )*);
            /// Every defined bit with its name, in bit order.
            pub const BITS: &'static [($name, &'static str)] = &[ $( ($name::$flag, $text), )* ];

            /// The raw bits.
            pub const fn bits(self) -> $repr {
                self.0
            }

            /// The set for `bits`, or `None` if a reserved bit is set.
            pub const fn from_bits(bits: $repr) -> Option<$name> {
                if bits & !Self::ALL.0 == 0 { Some($name(bits)) } else { None }
            }

            /// True if every bit of `other` is set in `self`.
            pub const fn contains(self, other: $name) -> bool {
                self.0 & other.0 == other.0
            }

            /// True if no bit is set.
            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }

            /// The merge of two facts: bitwise OR.
            pub const fn union(self, other: $name) -> $name {
                $name(self.0 | other.0)
            }
        }

        impl core::ops::BitOr for $name {
            type Output = $name;
            fn bitor(self, other: $name) -> $name {
                self.union(other)
            }
        }

        impl core::ops::BitOrAssign for $name {
            fn bitor_assign(&mut self, other: $name) {
                *self = self.union(other);
            }
        }

        // Bits are dense: BITS[i] is bit i.
        const _: () = {
            let all = $name::BITS;
            let mut i = 0;
            while i < all.len() {
                assert!(all[i].0.0 == 1 << i, "bits must be 0, 1, 2, ... in order");
                i += 1;
            }
            assert!(all.len() <= <$repr>::BITS as usize);
        };
    };
}

byte_enum! {
    /// Where the facts of a snapshot come from (H30 fact sources). On a conflict H30 decides
    /// per fact: RUNTIME, then PROFILED, then HINTED.
    pub enum FactSource {
        /// IC and feedback data of this run (T0, T1, baseline-AOT ICs).
        Runtime = 0 => "runtime",
        /// A `.ep9prof` training-run profile.
        Profiled = 1 => "profiled",
        /// TS hints (`.ep9h`, H37); unsound by design.
        Hinted = 2 => "hinted",
    }
}

byte_enum! {
    /// The state of an inline cache (H16): uninitialized, then monomorphic, then polymorphic
    /// (at most 4 cases), then megamorphic (sticky).
    pub enum IcState {
        /// No execution recorded.
        Uninitialized = 0 => "uninitialized",
        /// One case.
        Monomorphic = 1 => "monomorphic",
        /// 2 to 4 cases.
        Polymorphic = 2 => "polymorphic",
        /// More than 4 cases; the generic path runs.
        Megamorphic = 3 => "megamorphic",
    }
}

byte_enum! {
    /// The JS object kind of a shape. A fact never holds a VM cell kind id (kinds.rs: no
    /// kind id is persisted); the exporter maps the kind to this list.
    pub enum ObjectKind {
        /// An ordinary object (VM kind OBJECT).
        Ordinary = 0 => "ordinary",
        /// An Array exotic object.
        Array = 1 => "array",
        /// A function with bytecode or a builtin (FUNCTION).
        Function = 2 => "function",
        /// An Error instance.
        Error = 3 => "error",
        /// An array or string iterator.
        Iterator = 4 => "iterator",
        /// A Boolean, Number, String, Symbol or BigInt wrapper object.
        PrimitiveWrapper = 5 => "primitive-wrapper",
        /// A bound function exotic object.
        BoundFunction = 6 => "bound-function",
    }
}

byte_enum! {
    /// The H18 elements kinds. Transitions go only from a smaller to a larger general kind.
    pub enum ElementsKind {
        /// Int32 values, no holes.
        PackedInt32 = 0 => "packed-int32",
        /// Numbers as raw f64, no holes.
        PackedDouble = 1 => "packed-double",
        /// Any values, no holes.
        PackedElements = 2 => "packed-elements",
        /// Int32 values with holes.
        HoleyInt32 = 3 => "holey-int32",
        /// Numbers as raw f64 with holes.
        HoleyDouble = 4 => "holey-double",
        /// Any values with holes.
        HoleyElements = 5 => "holey-elements",
        /// Sparse: a dictionary.
        Dictionary = 6 => "dictionary",
    }
}

byte_enum! {
    /// The well-known symbols (ECMA-262 table 1 order, then `dispose` and `asyncDispose`).
    /// The same order as the VM's `WellKnownSymbol`.
    pub enum WellKnownSymbol {
        /// `Symbol.asyncIterator`
        AsyncIterator = 0 => "asyncIterator",
        /// `Symbol.hasInstance`
        HasInstance = 1 => "hasInstance",
        /// `Symbol.isConcatSpreadable`
        IsConcatSpreadable = 2 => "isConcatSpreadable",
        /// `Symbol.iterator`
        Iterator = 3 => "iterator",
        /// `Symbol.match`
        Match = 4 => "match",
        /// `Symbol.matchAll`
        MatchAll = 5 => "matchAll",
        /// `Symbol.replace`
        Replace = 6 => "replace",
        /// `Symbol.search`
        Search = 7 => "search",
        /// `Symbol.species`
        Species = 8 => "species",
        /// `Symbol.split`
        Split = 9 => "split",
        /// `Symbol.toPrimitive`
        ToPrimitive = 10 => "toPrimitive",
        /// `Symbol.toStringTag`
        ToStringTag = 11 => "toStringTag",
        /// `Symbol.unscopables`
        Unscopables = 12 => "unscopables",
        /// `Symbol.dispose`
        Dispose = 13 => "dispose",
        /// `Symbol.asyncDispose`
        AsyncDispose = 14 => "asyncDispose",
    }
}

byte_enum! {
    /// How many closures a `CreateClosure` site made: the optimizer may specialize on the
    /// one closure.
    pub enum ClosureCount {
        /// The site never ran.
        None = 0 => "none",
        /// Exactly one closure.
        One = 1 => "one",
        /// Two or more closures.
        Many = 2 => "many",
    }
}

bitset! {
    /// The types of the values a site saw (results, stored values, `typeof` operands).
    pub struct TypeSet(u16) {
        /// An int32 number.
        INT32 = 0 => "int32",
        /// A number that is not an int32 (a double, -0 or NaN).
        DOUBLE = 1 => "double",
        /// `undefined`.
        UNDEFINED = 2 => "undefined",
        /// `null`.
        NULL = 3 => "null",
        /// `true` or `false`.
        BOOLEAN = 4 => "boolean",
        /// A string.
        STRING = 5 => "string",
        /// A symbol.
        SYMBOL = 6 => "symbol",
        /// A BigInt.
        BIGINT = 7 => "bigint",
        /// An ordinary object.
        PLAIN_OBJECT = 8 => "plain-object",
        /// An Array.
        ARRAY = 9 => "array",
        /// A callable object.
        FUNCTION = 10 => "function",
        /// Any other object.
        OTHER_OBJECT = 11 => "other-object",
    }
}

bitset! {
    /// Operand and result kinds of a `BinaryOp` site (like V8's BinaryOperationHint, as a
    /// set).
    pub struct BinaryHint(u8) {
        /// Int32 operands and an int32 result.
        INT32 = 0 => "int32",
        /// Int32 operands and a result outside int32 (overflow, -0, a fraction).
        INT32_OVERFLOW = 1 => "int32-overflow",
        /// Number operands, at least one not an int32.
        NUMBER = 2 => "number",
        /// An `undefined`, `null` or boolean operand (ToNumber of an oddball).
        ODDBALL = 3 => "oddball",
        /// A string operand.
        STRING = 4 => "string",
        /// BigInt operands.
        BIGINT = 5 => "bigint",
        /// Any other operand (objects, symbols): the generic path.
        OTHER = 6 => "other",
    }
}

bitset! {
    /// Operand kinds of a `CompareOp` site (like V8's CompareOperationHint, as a set).
    pub struct CompareHint(u16) {
        /// Int32 operands.
        INT32 = 0 => "int32",
        /// Number operands, at least one not an int32.
        NUMBER = 1 => "number",
        /// Boolean operands.
        BOOLEAN = 2 => "boolean",
        /// `undefined` or `null` operands.
        NULLISH = 3 => "nullish",
        /// Atom (interned string) operands: equality by identity.
        ATOM = 4 => "atom",
        /// Other string operands.
        STRING = 5 => "string",
        /// Symbol operands.
        SYMBOL = 6 => "symbol",
        /// BigInt operands.
        BIGINT = 7 => "bigint",
        /// Object operands.
        RECEIVER = 8 => "receiver",
        /// Mixed or other operands: the generic path.
        OTHER = 9 => "other",
    }
}

bitset! {
    /// `for-in` cache use (V8's ForInHint as a set). The highest set bit is the hint.
    pub struct ForInHint(u8) {
        /// The receiver's enum cache had keys and field indices.
        KEYS_AND_INDICES = 0 => "enum-cache-keys-and-indices",
        /// The enum cache had keys only.
        KEYS = 1 => "enum-cache-keys",
        /// The generic path ran (dictionary, elements, prototype keys, proxies).
        ANY = 2 => "any",
    }
}

bitset! {
    /// Keys a keyed site saw.
    pub struct KeySet(u8) {
        /// An array index (a number or an index string).
        INDEX = 0 => "index",
        /// A string that is not an array index.
        NAME = 1 => "name",
        /// A symbol.
        SYMBOL = 2 => "symbol",
        /// Any other value: ToPropertyKey converts it.
        OTHER = 3 => "other",
    }
}

bitset! {
    /// The bindings a global site found.
    pub struct BindingSet(u8) {
        /// A `let`, `const` or `class` binding of the global lexical environment.
        LEXICAL = 0 => "lexical",
        /// A data property of the global object.
        DATA = 1 => "data-property",
        /// An accessor property of the global object.
        ACCESSOR = 2 => "accessor-property",
        /// No binding (ReferenceError, or `undefined` inside `typeof`).
        MISSING = 3 => "missing",
    }
}

bitset! {
    /// Property attributes in a shape. The same bits as the VM's descriptor details.
    pub struct PropAttrs(u8) {
        /// `[[Writable]]` (data properties).
        WRITABLE = 0 => "w",
        /// `[[Enumerable]]`.
        ENUMERABLE = 1 => "e",
        /// `[[Configurable]]`.
        CONFIGURABLE = 2 => "c",
        /// An accessor property (the slot holds the getter/setter pair).
        ACCESSOR = 3 => "accessor",
    }
}

bitset! {
    /// Shape flags. The same bits as the VM's shape flags 1 and 2.
    pub struct ShapeFlags(u8) {
        /// New properties may be added.
        EXTENSIBLE = 0 => "extensible",
        /// The properties live in a dictionary; the shape lists no keys.
        DICTIONARY = 1 => "dictionary",
    }
}

impl ForInHint {
    /// The effective hint: the highest set bit, or `None` if the site never ran.
    pub fn hint(self) -> Option<ForInHint> {
        Self::BITS
            .iter()
            .rev()
            .map(|&(b, _)| b)
            .find(|&b| self.contains(b))
    }
}

/// The names of the set bits, as `{a,b}`.
pub(crate) fn bit_names<T: Copy + PartialEq>(
    bits: &[(T, &str)],
    contains: impl Fn(T) -> bool,
) -> String {
    let names: Vec<&str> = bits
        .iter()
        .filter(|(b, _)| contains(*b))
        .map(|&(_, n)| n)
        .collect();
    format!("{{{}}}", names.join(","))
}
