//! Byte enums and bitsets of the fact schema. Every value has a fixed byte or bit; the schema
//! text (`schema_text`) lists them all, so a change here changes the schema fingerprint.
//!
//! Bitsets only grow: two facts about one slot merge by OR (`|`), whatever their source. A
//! reserved bit is never set; the decoder rejects it.
//!
//! Open lists (FACT_SCHEMA 2; MC-w23-8, M6-PLAN-D4): a list that the VM appends to after the
//! M1 freeze is open, so an append moves no `FACT_SCHEMA_VERSION`. A reader never refuses a
//! snapshot for a value of an open list that it does not name: it reads an elements kind as
//! none ([`ElementsKind`]), a representation as any ([`FieldRep`]), and drops a bit of an open
//! set ([`ElementsKindSet`], [`ShapeFlags`]). The schema text lists their names apart from the
//! format (`schema_text`), and the schema fingerprint leaves them out.

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

            /// The set for `bits` with every bit this schema does not name dropped: the read
            /// of an open set (the module docs).
            pub const fn from_bits_truncate(bits: $repr) -> $name {
                $name(bits & Self::ALL.0)
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
    ///
    /// Keyed sites (LoadKeyed, StoreKeyed, DefineKeyedOwn, HasKeyed, StoreInArrayLiteral): the
    /// state is live only for StoreKeyed, whose fast case (a present element of an Array)
    /// writes the IC key half. At the other keyed kinds, and for a StoreKeyed site that never
    /// took that fast case, `Uninitialized` means "no state", not "never ran". A keyed
    /// `Monomorphic` exports no case (MC-w15-1).
    pub enum IcState {
        /// No execution recorded (at a keyed site: no IC state; see above).
        Uninitialized = 0 => "uninitialized",
        /// One case (at a keyed site: one receiver shape, exported with no case).
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
        /// An arguments exotic object (VM kind ARGUMENTS; FACT_SCHEMA 2). With the shape's
        /// elements kind it is the fact kind of an arguments object (M6-PLAN-D5).
        Arguments = 7 => "arguments",
        /// A typed array (FACT_SCHEMA 2, declared): with the shape's elements kind it is the
        /// fact kind of a typed array (M6-PLAN-D5). The VM has no typed array kind yet, so no
        /// exporter writes it.
        TypedArray = 8 => "typed-array",
    }
}

byte_enum! {
    /// The H18 elements kinds, by their VM kind value (FACT_SCHEMA 2: the VM order,
    /// `elements::kind`; schema 1 put the holey kinds after the packed ones). Transitions go
    /// only from a smaller to a larger general kind.
    ///
    /// An open list (the module docs): the values are the H18 part "elements kind values
    /// (append-only)", 0-15 (4 shape bits); 7-15 are free. Every elements-kind field is a byte
    /// `value + 1` (0 = none), and a reader reads a value it does not name as none for that
    /// field only, never as a refusal of the snapshot (MC-w23-8). The fact kinds of typed
    /// arrays, arguments objects and string wrappers take no value here: they are the pair
    /// (`ShapeDesc::kind`, `ShapeDesc::elements`) (M6-PLAN-D5).
    pub enum ElementsKind {
        /// Int32 values, no holes.
        PackedInt32 = 0 => "packed-int32",
        /// Int32 values with holes.
        HoleyInt32 = 1 => "holey-int32",
        /// Numbers as raw f64, no holes.
        PackedDouble = 2 => "packed-double",
        /// Numbers as raw f64 with holes.
        HoleyDouble = 3 => "holey-double",
        /// Any values, no holes.
        PackedElements = 4 => "packed-elements",
        /// Any values with holes.
        HoleyElements = 5 => "holey-elements",
        /// Sparse: a dictionary.
        Dictionary = 6 => "dictionary",
    }
}

impl ElementsKind {
    /// The values an elements-kind field can name: 0-15, the 4 shape bits (H17 layout core).
    pub const VALUES: u8 = 16;

    /// The read of an open elements-kind value (the module docs): the kind, or `None` for a
    /// value this schema does not name.
    pub const fn from_open(v: u8) -> Option<ElementsKind> {
        ElementsKind::from_u8(v)
    }
}

/// The elements kinds a keyed site saw (FACT_SCHEMA 2; M6-PLAN-D4): bit `i` is the elements
/// kind of value `i` (0-15), the same bits as the IC_LAYOUT 4 kinds set of a keyed slot
/// (`elements::KINDS_SET_MASK`, shifted down by 10). An open set: a reader drops the bit of a
/// value it does not name ([`ElementsKind::from_open`]). Merges by OR.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElementsKindSet(u16);

impl ElementsKindSet {
    /// No kind.
    pub const EMPTY: ElementsKindSet = ElementsKindSet(0);

    /// The set of the one kind `k`.
    pub const fn of(k: ElementsKind) -> ElementsKindSet {
        ElementsKindSet(1 << k as u8)
    }

    /// The raw bits.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// The set for `bits`, the bits of the kinds this schema does not name dropped (the read
    /// of an open set).
    pub const fn from_bits_truncate(bits: u16) -> ElementsKindSet {
        let mut known = 0u16;
        let mut i = 0;
        while i < ElementsKind::ALL.len() {
            known |= 1 << ElementsKind::ALL[i] as u8;
            i += 1;
        }
        ElementsKindSet(bits & known)
    }

    /// True if `k` is in the set.
    pub const fn contains(self, k: ElementsKind) -> bool {
        self.0 & (1 << k as u8) != 0
    }

    /// True if no kind is in the set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The merge of two facts: bitwise OR.
    pub const fn union(self, other: ElementsKindSet) -> ElementsKindSet {
        ElementsKindSet(self.0 | other.0)
    }

    /// The kinds in the set, in value order.
    pub fn kinds(self) -> impl Iterator<Item = ElementsKind> {
        ElementsKind::ALL
            .iter()
            .copied()
            .filter(move |&k| self.contains(k))
    }
}

impl core::ops::BitOr for ElementsKindSet {
    type Output = ElementsKindSet;
    fn bitor(self, other: ElementsKindSet) -> ElementsKindSet {
        self.union(other)
    }
}

impl core::ops::BitOrAssign for ElementsKindSet {
    fn bitor_assign(&mut self, other: ElementsKindSet) {
        *self = self.union(other);
    }
}

byte_enum! {
    /// The representation hint of a field (FACT_SCHEMA 2; M6-PLAN-D8): the VM's details bits
    /// 4-6 (`shape::rep`, H17 "representation values (append-only)"). A hint: a compiled reader
    /// still guards (the shape guard stays the source). An open list: a reader reads a value it
    /// does not name (5-7 are free) as [`FieldRep::Any`], the hint that claims nothing.
    pub enum FieldRep {
        /// No value stored yet (the VM's `rep::NONE`).
        Unset = 0 => "unset",
        /// Every value stored was an int32.
        Int32 = 1 => "int32",
        /// Every value stored was a number.
        Double = 2 => "double",
        /// No value stored was a number.
        Heap = 3 => "heap",
        /// Any value.
        Any = 4 => "any",
    }
}

impl FieldRep {
    /// The read of an open representation value (3 bits): unknown values are `Any`.
    pub const fn from_open(v: u8) -> FieldRep {
        match FieldRep::from_u8(v) {
            Some(r) => r,
            None => FieldRep::Any,
        }
    }
}

/// The representation and const hint of one key of a shape (FACT_SCHEMA 2; M6-PLAN-D8): the
/// VM's details bits 4-7 for the key (`shape::rep`, `shape::DETAILS_CONST`). Encoded with the
/// key's [`PropAttrs`] in one byte, as the VM's details bits 0-7.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FieldHint {
    /// The representation of the values stored.
    pub rep: FieldRep,
    /// No object of the shape's family stored a second value into the field.
    pub constant: bool,
}

// `byte_enum!` derives a fixed trait list, so the default is written out here.
#[allow(clippy::derivable_impls)]
impl Default for FieldRep {
    fn default() -> FieldRep {
        FieldRep::Unset
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
    /// What the element accesses of a keyed site found (FACT_SCHEMA 2; MC-w13-9, MODES-D18):
    /// the receiver kinds and the access results, the VM's keyed slot bits 4-9
    /// (`profile::keyed`) shifted down by 4. Merges by OR.
    pub struct KeyedHint(u8) {
        /// An Array receiver.
        ARRAY = 0 => "array",
        /// Another object receiver.
        OBJECT = 1 => "object",
        /// A string receiver.
        STRING = 2 => "string",
        /// Another primitive receiver (number, boolean, symbol, undefined, null).
        PRIMITIVE = 3 => "primitive",
        /// An index key at or past an Array's length.
        OUT_OF_BOUNDS = 4 => "out-of-bounds",
        /// An index key below an Array's length whose element is a hole.
        HOLE = 5 => "hole",
    }
}

bitset! {
    /// Per-record flags (FACT_SCHEMA 2), for every slot kind. Merge by OR.
    pub struct RecordFlags(u8) {
        /// H29-Q5: a deopt exit of compiled code that guarded on this record's facts was
        /// taken, so a later compile does not speculate on them again.
        EXITED = 0 => "exited",
        /// MODES-D15: the site ran inside a sampled window, so an implicit bit (an int32 at a
        /// BinaryOp or CompareOp site, an int32 index of an Array at a LoadKeyed site) means
        /// "seen", not "seen or never ran" (F-R9). Only at the MODES-D17 sites: BinaryOp,
        /// CompareOp and LoadKeyed; at LoadKeyed only with the keyed hint bits (MODES-D18,
        /// [`KeyedHint`]). Clear means the site has no sampled run on record.
        RAN = 1 => "ran",
    }
}

bitset! {
    /// Shape flags. The same bits as the VM's shape flags 1 and 2. An open set (FACT_SCHEMA 2,
    /// the module docs): a later flag (the H18 extra element kinds take shape flag bits first,
    /// M6-PLAN-D6) is an append, and a reader drops a bit it does not name.
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
