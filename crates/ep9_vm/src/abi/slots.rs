//! Objects with internal slots on the builtin ABI (H20; M2-P3 ABI-SLOTS): the append-only
//! [`ObjectClass`] list and the `Cx` methods `new_object_with_slots`, `slot`, `set_slot` and
//! `object_class`.
//!
//! ```text
//! cx.new_object_with_slots(proto, ObjectClass::Map, 1)
//!   SLOT_OBJECT (kind 24, object::slot_obj)
//!     +24 ObjectClass id (int32)
//!     +32 ELEMENTS cell ── slot 0 .. slot n-1 (undefined at creation), or undefined when n = 0
//! cx.slot(o, ObjectClass::Map, 0)       RequireInternalSlot, then read slot 0
//! cx.set_slot(o, ObjectClass::Map, 0, v) RequireInternalSlot, then write slot 0 (barriered)
//! ```
//!
//! | Method | Spec | Errors |
//! |---|---|---|
//! | `new_object_with_slots(proto, class, n)` | OrdinaryObjectCreate(proto, internalSlotsList) (§10.1.12) | none; panics if `n` > [`ObjectClass::MAX_SLOTS`] |
//! | `slot(o, class, i)`, `set_slot(o, class, i, v)` | RequireInternalSlot(O, slot) (§10.1.15), then read or write | TypeError if `o` is not an object of `class`; panics if `i` ≥ the object's slot count |
//! | `object_class(v)` | "has a `[[PromiseState]]` internal slot" tests: IsPromise (§27.2.1.6), the builtinTag of `Object.prototype.toString` (§20.1.3.6) | none |
//!
//! The class decides what each slot index means; the builtin module of the class names its
//! indices (P4: `[[PromiseState]]` = 0, ...). Slots hold Values only; they live in a heap
//! cell, never in Rust memory (plan rule 8), so a stored object or string survives every
//! collection. Property code never touches the slots: `o.x = 1`, a dictionary move and
//! PreventExtensions leave them as they are.

use ep9_value::JsResult;

use super::{Cx, JsObject, Local};
use crate::conv;
use crate::frame::NULL;
use crate::kinds;
use crate::object;

/// The kinds of objects with internal slots (`[[MapData]]`, `[[PromiseState]]`, ...). The id
/// of a class is its position in this list: append-only, never reordered
/// (`crates/ep9_vm/tests/golden/object-classes.txt` is checked by `tests::slots`;
/// `EP9_BLESS=1` rewrites it after a review that sees only appended lines).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
#[non_exhaustive]
pub enum ObjectClass {
    /// A Promise instance: `[[PromiseState]]`, `[[PromiseResult]]`,
    /// `[[PromiseFulfillReactions]]`, `[[PromiseRejectReactions]]`, `[[PromiseIsHandled]]`.
    Promise,
    /// A PromiseReaction Record (§27.2.1.2: `[[Capability]]`, `[[Type]]`, `[[Handler]]`),
    /// kept in a heap cell (plan rule 8). Made with a null prototype; JS never sees one.
    PromiseReaction,
    /// A Date instance: `[[DateValue]]`.
    Date,
    /// A Map instance: `[[MapData]]`.
    Map,
    /// A Set instance: `[[SetData]]`.
    Set,
    /// An ArrayBuffer instance: `[[ArrayBufferData]]`, `[[ArrayBufferByteLength]]`,
    /// `[[ArrayBufferMaxByteLength]]`, `[[ArrayBufferDetachKey]]`. Slots hold Values: the
    /// byte store itself needs a native-data addition (not in M0 month 2).
    ArrayBuffer,
    /// A RegExp instance: `[[OriginalSource]]`, `[[OriginalFlags]]`, `[[RegExpRecord]]` (bit 8:
    /// `[[LegacyFeaturesEnabled]]`, M5-P2), `[[RegExpMatcher]]`, `[[Realm]]` (Annex B, M5-P2;
    /// the `regexp.rs` slot table).
    RegExp,
    /// A WeakMap instance: `[[WeakMapData]]` (M3-P1).
    WeakMap,
    /// A WeakSet instance: `[[WeakSetData]]` (M3-P1).
    WeakSet,
    /// A Map Iterator (§24.1.5): the table it walks, the next index, the kind (M3-P1).
    MapIterator,
    /// A Set Iterator (§24.2.6): the table it walks, the next index, the kind (M3-P1).
    SetIterator,
    /// A WeakRef instance (§26.1): `[[WeakRefTarget]]` in a WEAK_CELL (kind 80). Declared in
    /// M4-P0 (V9-D23); made by M4-P4 WEAKREF.
    WeakRef,
    /// A FinalizationRegistry instance (§26.2): `[[CleanupCallback]]`, `[[Cells]]` (WEAK_CELL
    /// entries). Declared in M4-P0 (V9-D23); made by M4-P4 WEAKREF.
    FinalizationRegistry,
    /// A RegExp String Iterator (§22.2.9): `[[IteratingRegExp]]`, `[[IteratedString]]`,
    /// `[[Global]]`, `[[Unicode]]`, `[[Done]]`. Declared in M4-P0; made by M4-P3 REGEXP1.
    RegExpStringIterator,
    /// A raw JSON object (`JSON.rawJSON`, json-parse-with-source): `[[IsRawJSON]]`. The class
    /// is the mark; the object has 0 slots. Appended by M6-P1 SPEC-B.
    RawJson,
    /// A Temporal.Duration instance (spec-temporal T2): `[[Years]]` .. `[[Nanoseconds]]`, slots 0-9, each an integral Number (`ep9_temporal` duration.rs).
    TemporalDuration,
    /// A Temporal.Instant instance (spec-temporal T3): `[[EpochNanoseconds]]` as slot 0 the epoch seconds (an integral Number) and slot 1 the nanosecond of that second (an int32), `ep9_temporal` instant.rs.
    TemporalInstant,
    /// A Temporal.PlainTime instance (spec-temporal T4): `[[Time]]` as slot 0 the second of the day (0..86,399, an int32) and slot 1 the nanosecond of that second (an int32), `ep9_temporal` plain_time.rs.
    TemporalPlainTime,
    /// A Temporal.PlainDate instance (spec-temporal T5): `[[ISODate]]` as slots 0-2 (ISO year, month, day: int32s) and `[[Calendar]]` as slot 3 (an ep9 calendar code, 0 = iso8601), `ep9_temporal` plain_date.rs.
    TemporalPlainDate,
    /// A Temporal.PlainDateTime instance (spec-temporal T6): `[[ISODateTime]]` as slots 0-2 (ISO year, month, day) and 4-5 (the second of the day, the nanosecond of that second), `[[Calendar]]` as slot 3 (an ep9 calendar code, 0 = iso8601); all int32s, `ep9_temporal` plain_date_time.rs.
    TemporalPlainDateTime,
    /// A Temporal.PlainYearMonth instance (spec-temporal T7): `[[ISODate]]` as slots 0-2 (ISO year, month, reference day) and `[[Calendar]]` as slot 3 (an ep9 calendar code); all int32s, `ep9_temporal` plain_year_month.rs.
    TemporalPlainYearMonth,
    /// A Temporal.PlainMonthDay instance (spec-temporal T7): `[[ISODate]]` as slots 0-2 (reference ISO year, month, day) and `[[Calendar]]` as slot 3 (an ep9 calendar code); all int32s, `ep9_temporal` plain_month_day.rs.
    TemporalPlainMonthDay,
    /// A Temporal.ZonedDateTime instance (spec-temporal T8): `[[EpochNanoseconds]]` as slot 0 the epoch seconds (an integral Number) and slot 1 the nanosecond of that second (an int32), `[[TimeZone]]` as slot 2 (the time zone identifier, a String) and `[[Calendar]]` as slot 3 (an ep9 calendar code, an int32), `ep9_temporal` zoned_date_time.rs.
    TemporalZonedDateTime,
    /// An Iterator Helper object (§27.1.2.1, M7-P1 SPEC-ITER), or the wrapper of
    /// `Iterator.from` (`[[Iterated]]`): slot 0 the kind and the GeneratorState (an int32),
    /// slots 1-2 the underlying iterator and its `next`, then the callback, the counter and
    /// the inner iterator of flatMap (`ep9_builtins` iterator.rs).
    IteratorHelper,
    /// A DisposableStack instance (explicit resource management, M7-P3 SPEC-ERM): slot 0
    /// `[[DisposableState]]` (true = disposed), slot 1 `[[DisposeCapability]]` (the resource
    /// list, an internal array; `ep9_builtins` disposable.rs).
    DisposableStack,
    /// An AsyncDisposableStack instance (M7-P3 SPEC-ERM): slot 0 `[[AsyncDisposableState]]`,
    /// slot 1 `[[DisposeCapability]]`, as [`ObjectClass::DisposableStack`].
    AsyncDisposableStack,
    /// An Intl.Locale instance (ECMA-402 §14, INTL W1): slot 0 `[[Locale]]` (the canonical
    /// tag, a String); the other slots (`[[Calendar]]` .. `[[Numeric]]`) are named in
    /// `ep9_intl` locale.rs.
    IntlLocale,
    /// An Intl.Collator instance (ECMA-402 §10, INTL W1): slot 0 `[[Locale]]` (a String); the
    /// other slots (1 the packed options with `[[Collation]]`, 2 `[[BoundCompare]]`) are named
    /// in `ep9_intl` collator.rs. No ICU4X object is in a slot (INTL-I1-D7).
    IntlCollator,
    /// A SharedArrayBuffer instance (§25.2; TypedArray TA-0 TA-DECL, design TA-D3): the slot
    /// form of [`ObjectClass::ArrayBuffer`] (`docs/plan/notes/typedarray-design.md` §2.2); its
    /// bytes are a shared block off the heap, never detached. Made by the `Cx` methods of
    /// `abi/typed.rs` (TA-1 TA-VM fills them).
    SharedArrayBuffer,
    /// A DataView instance (§25.3; TA-0 TA-DECL, TA-D3): `[[ViewedArrayBuffer]]`,
    /// `[[ByteOffset]]`, `[[ByteLength]]` and the cached data address (an `ExternalRef<u8>`
    /// native word, the H25 inventory) in its slots (design §2.2). Made by the `Cx` methods of
    /// `abi/typed.rs`.
    DataView,
}

impl ObjectClass {
    /// Every class, in id order.
    pub const ALL: &'static [ObjectClass] = &[
        ObjectClass::Promise,
        ObjectClass::PromiseReaction,
        ObjectClass::Date,
        ObjectClass::Map,
        ObjectClass::Set,
        ObjectClass::ArrayBuffer,
        ObjectClass::RegExp,
        ObjectClass::WeakMap,
        ObjectClass::WeakSet,
        ObjectClass::MapIterator,
        ObjectClass::SetIterator,
        ObjectClass::WeakRef,
        ObjectClass::FinalizationRegistry,
        ObjectClass::RegExpStringIterator,
        ObjectClass::RawJson,
        ObjectClass::TemporalDuration,
        ObjectClass::TemporalInstant,
        ObjectClass::TemporalPlainTime,
        ObjectClass::TemporalPlainDate,
        ObjectClass::TemporalPlainDateTime,
        ObjectClass::TemporalPlainYearMonth,
        ObjectClass::TemporalPlainMonthDay,
        ObjectClass::TemporalZonedDateTime,
        ObjectClass::IteratorHelper,
        ObjectClass::DisposableStack,
        ObjectClass::AsyncDisposableStack,
        ObjectClass::IntlLocale,
        ObjectClass::IntlCollator,
        ObjectClass::SharedArrayBuffer,
        ObjectClass::DataView,
    ];

    /// Most internal slots of one object.
    pub const MAX_SLOTS: usize = object::slot_obj::MAX_SLOTS;

    /// The id: the position in [`ObjectClass::ALL`] (stored as an int32 in the object).
    pub const fn id(self) -> u16 {
        self as u16
    }

    /// The class with id `id`, if there is one.
    pub const fn from_id(id: u16) -> Option<ObjectClass> {
        if (id as usize) < Self::ALL.len() {
            Some(Self::ALL[id as usize])
        } else {
            None
        }
    }

    /// The name (the spec name of the instance or record kind).
    pub const fn name(self) -> &'static str {
        match self {
            ObjectClass::Promise => "Promise",
            ObjectClass::PromiseReaction => "PromiseReaction",
            ObjectClass::Date => "Date",
            ObjectClass::Map => "Map",
            ObjectClass::Set => "Set",
            ObjectClass::ArrayBuffer => "ArrayBuffer",
            ObjectClass::RegExp => "RegExp",
            ObjectClass::WeakMap => "WeakMap",
            ObjectClass::WeakSet => "WeakSet",
            ObjectClass::MapIterator => "Map Iterator",
            ObjectClass::SetIterator => "Set Iterator",
            ObjectClass::WeakRef => "WeakRef",
            ObjectClass::FinalizationRegistry => "FinalizationRegistry",
            ObjectClass::RegExpStringIterator => "RegExp String Iterator",
            ObjectClass::RawJson => "RawJSON",
            ObjectClass::TemporalDuration => "Temporal.Duration",
            ObjectClass::TemporalInstant => "Temporal.Instant",
            ObjectClass::TemporalPlainTime => "Temporal.PlainTime",
            ObjectClass::TemporalPlainDate => "Temporal.PlainDate",
            ObjectClass::TemporalPlainDateTime => "Temporal.PlainDateTime",
            ObjectClass::TemporalPlainYearMonth => "Temporal.PlainYearMonth",
            ObjectClass::TemporalPlainMonthDay => "Temporal.PlainMonthDay",
            ObjectClass::TemporalZonedDateTime => "Temporal.ZonedDateTime",
            ObjectClass::IteratorHelper => "Iterator Helper",
            ObjectClass::DisposableStack => "DisposableStack",
            ObjectClass::AsyncDisposableStack => "AsyncDisposableStack",
            ObjectClass::IntlLocale => "Intl.Locale",
            ObjectClass::IntlCollator => "Intl.Collator",
            ObjectClass::SharedArrayBuffer => "SharedArrayBuffer",
            ObjectClass::DataView => "DataView",
        }
    }
}

impl<'s> Cx<'s> {
    /// OrdinaryObjectCreate(proto, internalSlotsList) (§10.1.12) for an object of `class`
    /// with `n` internal slots, all undefined. `proto` is an object or null (a
    /// PromiseReaction: null). For a constructor, `proto` comes from
    /// [`Cx::get_prototype_from_constructor`] (OrdinaryCreateFromConstructor, §10.1.13).
    /// Panics if `n` > [`ObjectClass::MAX_SLOTS`] or `proto` is not an object or null.
    pub fn new_object_with_slots(
        &mut self,
        proto: impl Into<Local<'s>>,
        class: ObjectClass,
        n: usize,
    ) -> Local<'s, JsObject> {
        let p = proto.into().bits();
        assert!(
            p == NULL || kinds::is_object(p),
            "a prototype is an object or null"
        );
        Self::local(object::new_slot_object(self.rt(), p, class.id(), n))
    }

    /// The class of `v` if it is an object with internal slots, else `None` (any value; never
    /// throws). For the spec tests "if `v` has a `[[PromiseState]]` internal slot".
    pub fn object_class(&self, v: impl Into<Local<'s>>) -> Option<ObjectClass> {
        object::slot_class(v.into().bits()).and_then(ObjectClass::from_id)
    }

    /// RequireInternalSlot(o, …) (§10.1.15), then internal slot `i` of `o`: a TypeError if `o`
    /// is not an object of `class`. Panics if `i` ≥ the slot count the object was made with
    /// (a bug in the builtin).
    pub fn slot(
        &mut self,
        o: impl Into<Local<'s>>,
        class: ObjectClass,
        i: usize,
    ) -> JsResult<Local<'s>> {
        let o = self.require_class(o.into(), class)?;
        Ok(Self::local(object::slot_get(o, i)))
    }

    /// RequireInternalSlot(o, …) (§10.1.15), then sets internal slot `i` of `o` to `v`: a
    /// TypeError if `o` is not an object of `class`. Panics if `i` ≥ the slot count the object
    /// was made with (a bug in the builtin).
    pub fn set_slot(
        &mut self,
        o: impl Into<Local<'s>>,
        class: ObjectClass,
        i: usize,
        v: impl Into<Local<'s>>,
    ) -> JsResult<()> {
        let o = self.require_class(o.into(), class)?;
        object::slot_set(self.rt(), o, i, v.into().bits());
        Ok(())
    }

    /// The bits of `o` if it is an object of `class`, else a TypeError.
    fn require_class(&mut self, o: Local<'s>, class: ObjectClass) -> JsResult<u64> {
        let b = o.bits();
        if object::slot_class(b) == Some(class.id()) {
            return Ok(b);
        }
        let msg = format!("{} is not a {} object", conv::describe(b), class.name());
        Err(self.throw_type_error(&msg))
    }
}
