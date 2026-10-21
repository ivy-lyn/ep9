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
    /// A RegExp instance: `[[OriginalSource]]`, `[[OriginalFlags]]`, `[[RegExpRecord]]`,
    /// `[[RegExpMatcher]]`.
    RegExp,
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
