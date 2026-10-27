//! Map and Set storage on the builtin ABI (owner M3-P1 B-COLLECTIONS; V8-C3): the `Cx`
//! methods over ORDERED_TABLE cells (`ordered_table.rs`) that `ep9_builtins` collections.rs
//! calls. A builtin never sees a table: it names the collection object and its class.
//!
//! ```text
//! cx.new_collection(proto, ObjectClass::Map)      SLOT_OBJECT, slot 0 = an empty table
//! cx.collection_set(map, ObjectClass::Map, k, v)  RequireInternalSlot, then the table op
//! cx.collection_cursor(map, class) + collection_next(&mut cursor)    forEach, Set methods
//! cx.new_collection_iterator(map, class, k, v)    a Map Iterator / Set Iterator object
//! cx.collection_iterator_next(it, class)          one step of %MapIteratorPrototype%.next
//! ```
//!
//! | Method | Spec | Errors |
//! |---|---|---|
//! | `new_collection(proto, class)` | OrdinaryObjectCreate(proto, « `[[MapData]]` ») and the others, with an empty List | none; panics for a class that is not Map, Set, WeakMap or WeakSet |
//! | `collection_get`, `collection_has`, `collection_set`, `collection_delete`, `collection_clear`, `collection_size` | RequireInternalSlot (§10.1.15), then the List step of §24.1-§24.4: SameValueZero keys, -0 stored as +0 | TypeError for another receiver; `collection_set`: RangeError past 2^24 entries |
//! | `collection_cursor`, `collection_next` | the index walks of `forEach` (§24.1.3.5, §24.2.4.8) and the Set methods (§24.2.4): entries added during the walk are seen, deleted ones skipped | `collection_cursor`: TypeError for another receiver |
//! | `new_collection_iterator(o, class, keys, values)` | CreateMapIterator (§24.1.5.1), CreateSetIterator (§24.2.6.1) | TypeError for another receiver |
//! | `collection_iterator_next(it, class)` | %MapIteratorPrototype%.next (§24.1.5.2.1), %SetIteratorPrototype%.next (§24.2.6.2.1): GeneratorResume of the closure | TypeError if `it` is not an iterator of `class` |
//! | `can_be_held_weakly(v)` | CanBeHeldWeakly (§9.13) | none |
//!
//! A key is compared by SameValueZero and hashed without an address (H12; the identity hash
//! of an object is H12-Q1, `ordered_table::identity_hash`). A cursor holds a table in a
//! [`Local`], so it is stack-only (H20 R2), like an `IteratorRecord`.

use ep9_value::JsResult;

use super::{Cx, JsObject, Local, ObjectClass};
use crate::builtins::IntrinsicId;
use crate::conv;
use crate::frame::{UNDEFINED, int32_bits, int32_of};
use crate::object;
use crate::ordered_table::{self as table, MAP_WIDTH, SET_WIDTH};
use crate::symbol;

/// The `kind` of a Map or Set iterator (CreateMapIterator, §24.1.5.1), as stored in its
/// slot: the key, the value, or `[key, value]`.
mod iter_kind {
    pub const KEYS: i32 = 0;
    pub const VALUES: i32 = 1;
    pub const ENTRIES: i32 = 2;
}

/// The internal slots of a Map Iterator or Set Iterator object: the table it walks
/// (undefined once it is done), the next entry index and the kind.
mod iter_slot {
    pub const TABLE: usize = 0;
    pub const INDEX: usize = 1;
    pub const KIND: usize = 2;
    pub const COUNT: usize = 3;
}

/// A walk over the entries of a Map or Set ([`Cx::collection_cursor`]): it sees the entries
/// added during the walk and skips the deleted ones, also across a rehash or a `clear`.
/// Stack-only (H20 R2): keep it in a Rust local, never in Rust heap memory.
#[derive(Debug)]
pub struct CollectionCursor<'s> {
    table: Local<'s>,
    index: usize,
    /// A Set yields its key as the value.
    set: bool,
}

/// The entry width and the weak flag of the tables of `class`. Panics for a class that is
/// not a collection (a bug in the builtin).
fn table_kind(class: ObjectClass) -> (usize, bool) {
    match class {
        ObjectClass::Map => (MAP_WIDTH, false),
        ObjectClass::Set => (SET_WIDTH, false),
        ObjectClass::WeakMap => (MAP_WIDTH, true),
        ObjectClass::WeakSet => (SET_WIDTH, true),
        _ => panic!("{} is not a collection class", class.name()),
    }
}

impl<'s> Cx<'s> {
    /// The object `o` if it is a collection object of `class` (RequireInternalSlot), else a
    /// TypeError.
    fn collection(&mut self, o: Local<'s>, class: ObjectClass) -> JsResult<u64> {
        table_kind(class);
        let b = o.bits();
        if object::slot_class(b) == Some(class.id()) {
            return Ok(b);
        }
        let msg = format!(
            "Method {}.prototype function called on incompatible receiver {}",
            class.name(),
            conv::describe(b)
        );
        Err(self.throw_type_error(&msg))
    }

    /// OrdinaryObjectCreate(proto, « `[[MapData]]` ») (or `[[SetData]]`, `[[WeakMapData]]`,
    /// `[[WeakSetData]]`) for a collection of `class` (Map, Set, WeakMap or WeakSet), with an
    /// empty List. For a constructor, `proto` comes from
    /// [`Cx::get_prototype_from_constructor`]. Panics for another class or if `proto` is not
    /// an object or null.
    pub fn new_collection(
        &mut self,
        proto: impl Into<Local<'s>>,
        class: ObjectClass,
    ) -> Local<'s, JsObject> {
        let (width, weak) = table_kind(class);
        let o = self.new_object_with_slots(proto, class, 1).bits();
        let rt = self.rt();
        // GC point: `o` lives in a Rust local (CSS).
        let t = table::new_empty(rt, width, weak);
        object::slot_set(rt, o, table::TABLE_SLOT, t);
        Self::local(o)
    }

    /// The value of the entry with key `key` (SameValueZero) of the collection `o` of `class`
    /// (Map or WeakMap), or `None`: Map.prototype.get (§24.1.3.6) step 4. A TypeError if `o`
    /// is not a collection of `class`.
    pub fn collection_get(
        &mut self,
        o: impl Into<Local<'s>>,
        class: ObjectClass,
        key: impl Into<Local<'s>>,
    ) -> JsResult<Option<Local<'s>>> {
        let o = self.collection(o.into(), class)?;
        let t = table::table_of(o);
        Ok(table::lookup(self.rt(), t, key.into().bits())
            .map(|i| Self::local(table::value_at(t, i))))
    }

    /// True if the collection `o` of `class` has an entry with key `key` (SameValueZero):
    /// Map.prototype.has (§24.1.3.7) and the others. A TypeError if `o` is not a collection
    /// of `class`.
    pub fn collection_has(
        &mut self,
        o: impl Into<Local<'s>>,
        class: ObjectClass,
        key: impl Into<Local<'s>>,
    ) -> JsResult<bool> {
        let o = self.collection(o.into(), class)?;
        let t = table::table_of(o);
        Ok(table::lookup(self.rt(), t, key.into().bits()).is_some())
    }

    /// Sets the value of `key` to `value` in the collection `o` of `class`, or appends the
    /// entry (a -0 key is stored as +0): Map.prototype.set (§24.1.3.9) steps 3-6 and
    /// Set.prototype.add (§24.2.4.1) (the value is ignored for a Set or WeakSet). A weak
    /// collection takes only a key for which [`Cx::can_be_held_weakly`] holds (the caller
    /// checks it; this panics otherwise). A TypeError if `o` is not a collection of `class`,
    /// a RangeError past 2^24 entries.
    pub fn collection_set(
        &mut self,
        o: impl Into<Local<'s>>,
        class: ObjectClass,
        key: impl Into<Local<'s>>,
        value: impl Into<Local<'s>>,
    ) -> JsResult<()> {
        let o = self.collection(o.into(), class)?;
        let (key, value) = (key.into(), value.into());
        assert!(
            !table_kind(class).1 || self.can_be_held_weakly(key),
            "a weak collection key must be an object or a symbol that is not registered"
        );
        // GC point: `o`, `key` and `value` live in Rust locals (CSS).
        if table::set(self.rt(), o, key.bits(), value.bits()).is_err() {
            let msg = format!("{} maximum size exceeded", class.name());
            return Err(self.throw_range_error(&msg));
        }
        Ok(())
    }

    /// Deletes the entry with key `key` (SameValueZero) of the collection `o` of `class`:
    /// true if there was one. Map.prototype.delete (§24.1.3.3) and the others. A TypeError if
    /// `o` is not a collection of `class`.
    pub fn collection_delete(
        &mut self,
        o: impl Into<Local<'s>>,
        class: ObjectClass,
        key: impl Into<Local<'s>>,
    ) -> JsResult<bool> {
        let o = self.collection(o.into(), class)?;
        Ok(table::delete(self.rt(), o, key.into().bits()))
    }

    /// Deletes every entry of the collection `o` of `class`: Map.prototype.clear
    /// (§24.1.3.1), Set.prototype.clear (§24.2.4.2). A TypeError if `o` is not a collection of
    /// `class`.
    pub fn collection_clear(
        &mut self,
        o: impl Into<Local<'s>>,
        class: ObjectClass,
    ) -> JsResult<()> {
        let o = self.collection(o.into(), class)?;
        // GC point: `o` lives in a Rust local.
        table::clear(self.rt(), o);
        Ok(())
    }

    /// The entry count of the collection `o` of `class` (Map or Set): `get size` (§24.1.3.10,
    /// §24.2.4.14), SetDataSize. A TypeError if `o` is not a collection of `class`.
    pub fn collection_size(
        &mut self,
        o: impl Into<Local<'s>>,
        class: ObjectClass,
    ) -> JsResult<usize> {
        let o = self.collection(o.into(), class)?;
        Ok(table::live_count(table::table_of(o)))
    }

    /// A cursor at the first entry of the collection `o` of `class` (Map or Set). A TypeError
    /// if `o` is not a collection of `class`.
    pub fn collection_cursor(
        &mut self,
        o: impl Into<Local<'s>>,
        class: ObjectClass,
    ) -> JsResult<CollectionCursor<'s>> {
        let o = self.collection(o.into(), class)?;
        Ok(CollectionCursor {
            table: Self::local(table::table_of(o)),
            index: 0,
            set: table_kind(class).0 == SET_WIDTH,
        })
    }

    /// The next entry of the cursor `c` as (key, value) (a Set gives its key twice), or
    /// `None` at the end. The cursor stays at the end, so a later call sees an entry added
    /// since. Allocates nothing.
    pub fn collection_next(
        &mut self,
        c: &mut CollectionCursor<'s>,
    ) -> Option<(Local<'s>, Local<'s>)> {
        let mut t = c.table.bits();
        let found = table::cursor_next(&mut t, &mut c.index);
        c.table = Self::local(t);
        let i = found?;
        let k = table::key_at(t, i);
        let v = if c.set { k } else { table::value_at(t, i) };
        Some((Self::local(k), Self::local(v)))
    }

    /// CreateMapIterator(map, kind) (§24.1.5.1) for a Map `o`, CreateSetIterator(set, kind)
    /// (§24.2.6.1) for a Set: a Map Iterator or Set Iterator object with the realm's
    /// `%MapIteratorPrototype%` or `%SetIteratorPrototype%`. A TypeError if `o` is not a
    /// collection of `class`; panics for a weak class. The kind: `keys` alone gives the keys
    /// (`keys()`), `values` alone the values (`values()`), both `[key, value]` arrays
    /// (`entries()`); a Set's value is its key.
    pub fn new_collection_iterator(
        &mut self,
        o: impl Into<Local<'s>>,
        class: ObjectClass,
        keys: bool,
        values: bool,
    ) -> JsResult<Local<'s, JsObject>> {
        let kind = match (keys, values) {
            (true, false) => iter_kind::KEYS,
            (false, true) => iter_kind::VALUES,
            _ => iter_kind::ENTRIES,
        };
        let o = self.collection(o.into(), class)?;
        let (it_class, proto) = match class {
            ObjectClass::Map => (ObjectClass::MapIterator, IntrinsicId::MapIteratorPrototype),
            ObjectClass::Set => (ObjectClass::SetIterator, IntrinsicId::SetIteratorPrototype),
            _ => panic!("{} has no iterator", class.name()),
        };
        let proto = self.intrinsic(proto);
        // GC point: `o` lives in a Rust local.
        let it = self.new_object_with_slots(proto, it_class, iter_slot::COUNT);
        let rt = self.rt();
        let it = it.bits();
        object::slot_set(rt, it, iter_slot::TABLE, table::table_of(o));
        object::slot_set(rt, it, iter_slot::INDEX, int32_bits(0));
        object::slot_set(rt, it, iter_slot::KIND, int32_bits(kind));
        Ok(Self::local(it))
    }

    /// One step of a Map Iterator or Set Iterator `it` of `class` (`MapIterator` or
    /// `SetIterator`): %MapIteratorPrototype%.next (§24.1.5.2.1) and
    /// %SetIteratorPrototype%.next (§24.2.6.2.1), GeneratorResume of the CreateMapIterator
    /// closure. `Some(result value)`: the key, the value, or a new `[key, value]` Array
    /// (`[value, value]` for a Set); `None` once the iterator is done, for good. A TypeError
    /// if `it` is not an iterator of `class` (GeneratorValidate with the brand).
    pub fn collection_iterator_next(
        &mut self,
        it: impl Into<Local<'s>>,
        class: ObjectClass,
    ) -> JsResult<Option<Local<'s>>> {
        let it = it.into().bits();
        if object::slot_class(it) != Some(class.id())
            || !matches!(class, ObjectClass::MapIterator | ObjectClass::SetIterator)
        {
            let msg = format!(
                "Method {}.prototype.next called on incompatible receiver {}",
                class.name(),
                conv::describe(it)
            );
            return Err(self.throw_type_error(&msg));
        }
        let rt = self.rt();
        let mut t = object::slot_get(it, iter_slot::TABLE);
        if t == UNDEFINED {
            return Ok(None);
        }
        let mut i = int32_of(object::slot_get(it, iter_slot::INDEX)) as usize;
        let Some(e) = table::cursor_next(&mut t, &mut i) else {
            object::slot_set(rt, it, iter_slot::TABLE, UNDEFINED);
            return Ok(None);
        };
        object::slot_set(rt, it, iter_slot::TABLE, t);
        object::slot_set(rt, it, iter_slot::INDEX, int32_bits(i as i32));
        let kind = int32_of(object::slot_get(it, iter_slot::KIND));
        let k = Self::local(table::key_at(t, e));
        let v = if class == ObjectClass::SetIterator {
            k
        } else {
            Self::local(table::value_at(t, e))
        };
        Ok(Some(match kind {
            iter_kind::KEYS => k,
            iter_kind::VALUES => v,
            // GC point: `k` and `v` live in Rust locals.
            _ => self.create_array_from_list(&[k, v])?.value(),
        }))
    }

    /// CanBeHeldWeakly(v) (§9.13): an object, or a symbol that `Symbol.for` did not make
    /// (KeyForSymbol is undefined). A WeakMap key, a WeakSet value.
    pub fn can_be_held_weakly(&self, v: impl Into<Local<'s>>) -> bool {
        let v = v.into();
        v.is_object() || (v.is_symbol() && !symbol::is_registered(v.bits()))
    }
}
