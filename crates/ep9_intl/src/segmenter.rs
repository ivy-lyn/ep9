//! Intl.Segmenter (ECMA-402 §19; block 36): `%Intl.Segmenter%`, `%Intl.Segmenter.prototype%`,
//! `%IntlSegmentsPrototype%` and `%IntlSegmentIteratorPrototype%`, a module of the lazy group
//! "Intl" (INTL-I1-D8): its properties target only `%Intl%` and its own intrinsics.
//!
//! ```text
//! Intl.Segmenter {w, c}        %Intl.Segmenter%  prototype {}, supportedLocalesOf
//! Intl.Segmenter.prototype     %Intl.Segmenter.prototype%  constructor {w, c},
//!                                resolvedOptions {w, c}, segment {w, c},
//!                                [@@toStringTag] "Intl.Segmenter" {c}
//! (no global path)            %IntlSegmentsPrototype%  containing {w, c},
//!                                [@@iterator] {w, c}
//! (no global path)            %IntlSegmentIteratorPrototype%  [[Prototype]] %IteratorPrototype%,
//!                                next {w, c}, [@@toStringTag] "Segmenter String Iterator" {c}
//! slots:
//!   IntlSegmenter        0 [[Locale]] (a String), 1 [[SegmenterGranularity]] (an int32)
//!   IntlSegments         0 [[SegmentsSegmenter]], 1 [[SegmentsString]], 2 the boundary table
//!   IntlSegmentIterator  0 [[IteratingSegmenter]], 1 [[IteratedString]],
//!                        2 [[IteratedStringNextSegmentCodeUnitIndex]] (an int32),
//!                        3 the boundary table, 4 the table position of the next segment
//! segment(s) ─ Segments ─ containing(n) ─┐
//!                 └ [@@iterator] ─ next() ┴ boundary table (made at the first use):
//!     the code units of the string ─ cache::get_or_make(Service::Segmenter, data locale,
//!     granularity) ─> Rc<WordSegmenter | SentenceSegmenter> (grapheme: the static one)
//!     ─ segment_utf16 ─ one int32 per segment: end << 1 | isWordLike, in an internal Array
//! ```
//!
//! ICU4X: `icu_segmenter` with the dictionary for the complex scripts (no LSTM, no `auto`;
//! dep-guards check 26). No ICU4X object is in a slot (INTL-I1-D7, plan rule 8): a slot holds
//! the string and the boundary table, both GC values, so the GC keeps the string of a live
//! Segments or Segment Iterator object.
//!
//! The boundary table holds the end of each segment of the whole string, as FindBoundary
//! (19.8.1) finds them: an ICU4X iterator starts at the start of the string, and a complex
//! script run (Thai, Japanese) is segmented as one run. One pass makes the whole table, so a
//! full iteration is linear and `containing` is a binary search. The table is 8 bytes per
//! segment in the GC heap (a known cost: V8 and SpiderMonkey keep an ICU break iterator per
//! object instead, which rule 8 does not allow here).
//!
//! Interrupt polls while the table is made: the copy of the code units (one budget unit per
//! [`POLL_UNITS`]), the scan (after each boundary, 1 + segment length / [`POLL_UNITS`]
//! units) and the store loop (one per entry). The ICU4X work for one segment does not poll
//! ([`segment_ends`]).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-segmenter.txt`.

use std::rc::Rc;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsObject, JsResult, JsString, Local, ObjectClass, ObjectProps, PropAttrs,
    PropDef, PropKey, WellKnownSymbol,
};
use icu_locale_core::LanguageIdentifier;
use icu_segmenter::options::{SentenceBreakOptions, WordBreakInvariantOptions, WordBreakOptions};
use icu_segmenter::{GraphemeClusterSegmenter, SentenceSegmenter, WordSegmenter};

use crate::cache::{self, Service};
use crate::options::{
    POLL_UNITS, get_locale_matcher, get_options_object, get_string_option, units,
};
use crate::resolve;
use crate::tag;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 36;

/// The class of an Intl.Segmenter object.
pub const CLASS: ObjectClass = ObjectClass::IntlSegmenter;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;
/// Slot 1: `[[SegmenterGranularity]]`, a [`Granularity`] as an int32.
pub const SLOT_GRANULARITY: usize = 1;
/// The slot count of an Intl.Segmenter object.
pub const SLOTS: usize = 2;

/// The slots of a Segments object (ObjectClass::IntlSegments, 19.5).
mod segments_slot {
    /// `[[SegmentsSegmenter]]`: the Intl.Segmenter object.
    pub const SEGMENTER: usize = 0;
    /// `[[SegmentsString]]`: the String.
    pub const STRING: usize = 1;
    /// The boundary table (undefined until the first `containing`).
    pub const TABLE: usize = 2;
    /// The slot count.
    pub const COUNT: usize = 3;
}

/// The slots of a Segment Iterator object (ObjectClass::IntlSegmentIterator, 19.6).
mod iterator_slot {
    /// `[[IteratingSegmenter]]`: the Intl.Segmenter object.
    pub const SEGMENTER: usize = 0;
    /// `[[IteratedString]]`: the String.
    pub const STRING: usize = 1;
    /// `[[IteratedStringNextSegmentCodeUnitIndex]]`: an int32.
    pub const NEXT_INDEX: usize = 2;
    /// The boundary table (undefined until the first `next`, unless the Segments object had
    /// one).
    pub const TABLE: usize = 3;
    /// The index in the table of the segment that starts at the next index (an int32).
    pub const POSITION: usize = 4;
    /// The slot count.
    pub const COUNT: usize = 5;
}

/// Definition indices.
mod def {
    pub const SUPPORTED_LOCALES_OF: u16 = 1;
    pub const RESOLVED_OPTIONS: u16 = 2;
    pub const SEGMENT: u16 = 3;
    pub const CONTAINING: u16 = 4;
    pub const SEGMENTS_ITERATOR: u16 = 5;
    pub const NEXT: u16 = 6;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("Segmenter", 0, constructor), // 0 (19.1.1)
    BuiltinDef::method("supportedLocalesOf", 1, supported_locales_of), // 1 (19.2.2)
    BuiltinDef::method("resolvedOptions", 0, resolved_options), // 2 (19.3.2)
    BuiltinDef::method("segment", 1, segment),            // 3 (19.3.3)
    BuiltinDef::method("containing", 1, containing),      // 4 (19.5.2.1)
    BuiltinDef::method("[Symbol.iterator]", 0, segments_iterator), // 5 (19.5.2.2)
    BuiltinDef::method("next", 0, next),                  // 6 (19.6.2.1)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlSegmenter,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlSegmenterPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::IntlSegmentsPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
    IntrinsicDef {
        id: IntrinsicId::IntlSegmentIteratorPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::IteratorPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlSegmenter,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::IntlSegmenterPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(
                PropKey::Str("supportedLocalesOf"),
                def::SUPPORTED_LOCALES_OF,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IntlSegmenterPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlSegmenter,
                PropAttrs::METHOD,
            ),
            // The key order of V8 (node): resolvedOptions, then segment.
            PropDef::method(PropKey::Str("resolvedOptions"), def::RESOLVED_OPTIONS),
            PropDef::method(PropKey::Str("segment"), def::SEGMENT),
            // Intl.Segmenter.prototype [ @@toStringTag ] ({c}, 19.3.4).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.Segmenter"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IntlSegmentsPrototype,
        props: &[
            PropDef::method(PropKey::Str("containing"), def::CONTAINING),
            PropDef::method(
                PropKey::Symbol(WellKnownSymbol::ITERATOR),
                def::SEGMENTS_ITERATOR,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IntlSegmentIteratorPrototype,
        props: &[
            PropDef::method(PropKey::Str("next"), def::NEXT),
            // %IntlSegmentIteratorPrototype% [ @@toStringTag ] ({c}, 19.6.2.2).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Segmenter String Iterator"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("Segmenter"),
            IntrinsicId::IntlSegmenter,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "segmenter",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the segmentation -------------------------------------------------------------------------

/// `[[SegmenterGranularity]]` (Table 19 of §19.1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Granularity {
    /// "grapheme": extended grapheme clusters (UAX #29).
    Grapheme,
    /// "word": words (UAX #29, the dictionary for the complex scripts).
    Word,
    /// "sentence": sentences (UAX #29).
    Sentence,
}

/// The granularity names, in [`Granularity`] order.
const GRANULARITIES: [&str; 3] = ["grapheme", "word", "sentence"];

impl Granularity {
    /// The value of an int32 slot (0 is "grapheme").
    fn from_slot(v: i32) -> Granularity {
        match v {
            1 => Granularity::Word,
            2 => Granularity::Sentence,
            _ => Granularity::Grapheme,
        }
    }

    /// The option name.
    pub fn name(self) -> &'static str {
        GRANULARITIES[self as usize]
    }
}

/// The ICU4X content locale of `data_locale` (none if ICU4X cannot read it).
fn content_locale(data_locale: &str) -> Option<LanguageIdentifier> {
    LanguageIdentifier::try_from_str(data_locale).ok()
}

/// The cached ICU4X word segmenter of `data_locale` (dictionary for the complex scripts).
fn word_segmenter(data_locale: &str) -> Option<Rc<WordSegmenter>> {
    cache::get_or_make(
        Service::Segmenter,
        data_locale,
        Granularity::Word as u64,
        || {
            let langid = content_locale(data_locale);
            let mut o = WordBreakOptions::default();
            o.content_locale = langid.as_ref();
            WordSegmenter::try_new_dictionary(o).ok().or_else(|| {
                Some(
                    WordSegmenter::new_dictionary(WordBreakInvariantOptions::default())
                        .static_to_owned(),
                )
            })
        },
    )
}

/// The cached ICU4X sentence segmenter of `data_locale` (the locale tailoring, if the data
/// has one: "el" ends a sentence at ';').
fn sentence_segmenter(data_locale: &str) -> Option<Rc<SentenceSegmenter>> {
    cache::get_or_make(
        Service::Segmenter,
        data_locale,
        Granularity::Sentence as u64,
        || {
            let langid = content_locale(data_locale);
            let mut o = SentenceBreakOptions::default();
            o.content_locale = langid.as_ref();
            SentenceSegmenter::try_new(o)
                .ok()
                .or_else(|| Some(SentenceSegmenter::new(Default::default()).static_to_owned()))
        },
    )
}

/// The boundary table of `units` for `data_locale` and `granularity`: for each segment of the
/// string in order, `end << 1 | isWordLike`, where `end` is the code unit index after the
/// segment (the last one is the length) and `isWordLike` is 1 for a word-like segment of the
/// granularity "word" (a letter, number or ideograph segment; ICU4X `WordType` not `None`).
/// An empty string has no segment. Every end is below 2^29 (the string length limit), so
/// each entry fits an int32.
///
/// The ICU4X iterator is lazy: after each boundary, `poll(n)` gets the work of that segment,
/// `n` = 1 + its length / [`POLL_UNITS`] budget units. A `false` from `poll` stops the scan
/// (`None`). The ICU4X work for one segment does not poll: one grapheme, word or sentence,
/// or one complex-script run (Thai, Japanese: the dictionary segments the run in one step).
/// It is linear in its length, at most the string length limit ((1 << 29) - 24 code units).
pub fn segment_ends(
    data_locale: &str,
    granularity: Granularity,
    units: &[u16],
    poll: &mut dyn FnMut(usize) -> bool,
) -> Option<Vec<u32>> {
    let mut ends: Vec<u32> = Vec::new();
    let mut start = 0;
    let mut push = |end: usize, word_like: bool| {
        if end > 0 {
            ends.push((end as u32) << 1 | u32::from(word_like));
        }
        let n = 1 + (end - start) / POLL_UNITS;
        start = end;
        poll(n)
    };
    match granularity {
        Granularity::Grapheme => {
            for end in GraphemeClusterSegmenter::new().segment_utf16(units) {
                if !push(end, false) {
                    return None;
                }
            }
        }
        Granularity::Word => match word_segmenter(data_locale) {
            Some(s) => {
                for (end, t) in s.as_borrowed().segment_utf16(units).iter_with_word_type() {
                    if !push(end, t.is_word_like()) {
                        return None;
                    }
                }
            }
            None => {
                if !push(units.len(), false) {
                    return None;
                }
            }
        },
        Granularity::Sentence => match sentence_segmenter(data_locale) {
            Some(s) => {
                for end in s.as_borrowed().segment_utf16(units) {
                    if !push(end, false) {
                        return None;
                    }
                }
            }
            None => {
                if !push(units.len(), false) {
                    return None;
                }
            }
        },
    }
    Some(ends)
}

// ---- the builtins -----------------------------------------------------------------------------

/// Intl.Segmenter ( [ locales [ , options ] ] ) (19.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Intl.Segmenter requires 'new'"));
    };
    // 2-3. Let segmenter be ? OrdinaryCreateFromConstructor(NewTarget,
    //      "%Intl.Segmenter.prototype%", internalSlotsList).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::IntlSegmenterPrototype)?;
    let segmenter = cx.new_object_with_slots(proto, CLASS, SLOTS);
    // 4. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, args.get(0))?;
    // 5. Set options to ? GetOptionsObject(options).
    let options = get_options_object(cx, args.get(1))?;
    // 6-8. Let matcher be ? GetOption(options, "localeMatcher", string, « "lookup",
    //      "best fit" », "best fit").
    get_locale_matcher(cx, options)?;
    // 9-11. Let r be ResolveLocale(%Intl.Segmenter%.[[AvailableLocales]], requestedLocales,
    //       opt, %Intl.Segmenter%.[[RelevantExtensionKeys]] (« »), localeData); set
    //       segmenter.[[Locale]] to r.[[Locale]].
    let r = resolve::resolve_locale(cx, &requested, &[], &[], &|_, _| Vec::new())?;
    let locale = cx.new_string(&r.locale)?;
    cx.set_slot(segmenter, CLASS, SLOT_LOCALE, locale)?;
    // 12. Let granularity be ? GetOption(options, "granularity", string, « "grapheme",
    //     "word", "sentence" », "grapheme").
    let granularity =
        get_string_option(cx, options, "granularity", &GRANULARITIES, Some("grapheme"))?;
    // 13. Set segmenter.[[SegmenterGranularity]] to granularity.
    let g = GRANULARITIES
        .iter()
        .position(|n| Some(*n) == granularity.as_deref())
        .unwrap_or(0);
    cx.set_slot(segmenter, CLASS, SLOT_GRANULARITY, Local::int32(g as i32))?;
    // 14. Return segmenter.
    Ok(segmenter.into())
}

/// Intl.Segmenter.supportedLocalesOf ( locales [ , options ] ) (19.2.2).
fn supported_locales_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let availableLocales be %Intl.Segmenter%.[[AvailableLocales]].
    // 2. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, args.get(0))?;
    // 3. Return ? FilterLocales(availableLocales, requestedLocales, options).
    resolve::supported_locales(cx, &requested, args.get(1))
}

/// The `[[Locale]]` text and the granularity of the segmenter `o` (TypeError if `o` is not an
/// Intl.Segmenter: RequireInternalSlot(segmenter, `[[InitializedSegmenter]]`)).
fn read<'s>(cx: &mut Cx<'s>, o: Local<'s>) -> JsResult<(String, Granularity)> {
    let locale = cx.slot(o, CLASS, SLOT_LOCALE)?;
    let granularity = cx.slot(o, CLASS, SLOT_GRANULARITY)?;
    let locale = cx.to_rust_string(locale)?;
    Ok((
        locale,
        Granularity::from_slot(granularity.as_int32().unwrap_or(0)),
    ))
}

/// Intl.Segmenter.prototype.resolvedOptions ( ) (19.3.2).
fn resolved_options<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let segmenter be the this value; ? RequireInternalSlot(segmenter,
    //      [[InitializedSegmenter]]).
    let (locale, granularity) = read(cx, this)?;
    // 3. Let options be OrdinaryObjectCreate(%Object.prototype%).
    let o = cx.new_object();
    // 4. For each row of Table 20, in table order: CreateDataPropertyOrThrow(options, p, v).
    let v = cx.new_string(&locale)?;
    cx.create_data_property_or_throw(o, "locale", v)?;
    let v = cx.new_string(granularity.name())?;
    cx.create_data_property_or_throw(o, "granularity", v)?;
    // 5. Return options.
    Ok(o.into())
}

/// Intl.Segmenter.prototype.segment ( string ) (19.3.3).
fn segment<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let segmenter be the this value; ? RequireInternalSlot(segmenter,
    //      [[InitializedSegmenter]]).
    cx.slot(this, CLASS, SLOT_LOCALE)?;
    // 3. Let string be ? ToString(string).
    let string = cx.to_string(args.get(0))?;
    // 4. Return CreateSegmentsObject(segmenter, string) (19.5.1).
    let proto = cx.intrinsic(IntrinsicId::IntlSegmentsPrototype);
    let segments = cx.new_object_with_slots(proto, ObjectClass::IntlSegments, segments_slot::COUNT);
    cx.set_slot(
        segments,
        ObjectClass::IntlSegments,
        segments_slot::SEGMENTER,
        this,
    )?;
    cx.set_slot(
        segments,
        ObjectClass::IntlSegments,
        segments_slot::STRING,
        string,
    )?;
    Ok(segments.into())
}

/// The boundary table (an internal Array of int32s, [`segment_ends`]) of `string` for the
/// Intl.Segmenter `segmenter`. Each entry is a Value in the GC heap; the Array is never seen
/// by JS code (it is only in a slot), and every element is an own data property, so a read
/// never goes to `Array.prototype`.
fn make_table<'s>(
    cx: &mut Cx<'s>,
    segmenter: Local<'s>,
    string: Local<'s, JsString>,
) -> JsResult<Local<'s, JsObject>> {
    let (locale, granularity) = read(cx, segmenter)?;
    let data_locale = tag::remove_unicode_extension(&locale);
    let units = units(cx, string)?;
    // The scan polls after each boundary (`segment_ends`), with the work of the segment.
    let mut polled = Ok(());
    let ends = segment_ends(&data_locale, granularity, &units, &mut |n| {
        polled = (0..n).try_for_each(|_| cx.poll_interrupt());
        polled.is_ok()
    });
    polled?;
    let ends = ends.unwrap_or_default();
    let table: Local<'s, JsObject> = cx.new_array().into();
    for (i, e) in ends.iter().enumerate() {
        cx.poll_interrupt()?;
        cx.create_data_property_index_or_throw(table, i as u64, Local::int32(*e as i32))?;
    }
    Ok(table)
}

/// The string of a Segments or Segment Iterator object (its slot holds a String).
fn string_of<'s>(v: Local<'s>) -> Local<'s, JsString> {
    v.downcast::<JsString>()
        .expect("the string slot holds a String")
}

/// Entry `i` of a boundary table: (end, isWordLike).
fn entry<'s>(cx: &mut Cx<'s>, table: Local<'s, JsObject>, i: u64) -> JsResult<(usize, bool)> {
    let e = cx.get_index(table, i)?.as_int32().unwrap_or(0) as u32;
    Ok(((e >> 1) as usize, e & 1 != 0))
}

/// CreateSegmentDataObject ( segmenter, string, startIndex, endIndex ) (19.7.1), with the
/// isWordLike of the segment from the boundary table.
fn segment_data<'s>(
    cx: &mut Cx<'s>,
    segmenter: Local<'s>,
    string: Local<'s, JsString>,
    start: usize,
    end: usize,
    word_like: bool,
) -> JsResult<Local<'s>> {
    let (_, granularity) = read(cx, segmenter)?;
    // 5. Let result be OrdinaryObjectCreate(%Object.prototype%).
    let result = cx.new_object();
    // 6-7. Let segment be the substring of string from startIndex to endIndex;
    //      CreateDataPropertyOrThrow(result, "segment", segment).
    let segment = cx.substring(string, start, end)?;
    cx.create_data_property_or_throw(result, "segment", segment)?;
    // 8. CreateDataPropertyOrThrow(result, "index", 𝔽(startIndex)).
    cx.create_data_property_or_throw(result, "index", Local::int32(start as i32))?;
    // 9. CreateDataPropertyOrThrow(result, "input", string).
    cx.create_data_property_or_throw(result, "input", string)?;
    // 10-11. If granularity is "word", CreateDataPropertyOrThrow(result, "isWordLike",
    //        isWordLike).
    if granularity == Granularity::Word {
        cx.create_data_property_or_throw(result, "isWordLike", Local::boolean(word_like))?;
    }
    // 12. Return result.
    Ok(result.into())
}

/// %IntlSegmentsPrototype%.containing ( index ) (19.5.2.1).
fn containing<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    const C: ObjectClass = ObjectClass::IntlSegments;
    // 1-2. Let segments be the this value; ? RequireInternalSlot(segments,
    //      [[SegmentsSegmenter]]).
    // 3. Let segmenter be segments.[[SegmentsSegmenter]].
    let segmenter = cx.slot(this, C, segments_slot::SEGMENTER)?;
    // 4. Let string be segments.[[SegmentsString]].
    let string = string_of(cx.slot(this, C, segments_slot::STRING)?);
    // 5. Let len be the length of string.
    let len = cx.string_len(string);
    // 6. Let n be ? ToIntegerOrInfinity(index).
    let n = cx.to_integer_or_infinity(args.get(0))?;
    // 7. If n < 0 or n ≥ len, return undefined.
    if n < 0.0 || n >= len as f64 {
        return Ok(Local::undefined());
    }
    let n = n as usize;
    // 8-9. Let startIndex be FindBoundary(segmenter, string, n, before); let endIndex be
    //      FindBoundary(segmenter, string, n, after): the segment of the table whose end is
    //      the first one above n.
    let table = match cx.slot(this, C, segments_slot::TABLE)?.as_object() {
        Some(t) => t,
        None => {
            let t = make_table(cx, segmenter, string)?;
            cx.set_slot(this, C, segments_slot::TABLE, t)?;
            t
        }
    };
    let count = cx.get(table, "length")?.as_int32().unwrap_or(0) as u64;
    let (mut lo, mut hi) = (0u64, count);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if entry(cx, table, mid)?.0 > n {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    if lo >= count {
        // Not reached: the last end is len, which is above n.
        return Ok(Local::undefined());
    }
    let (end, word_like) = entry(cx, table, lo)?;
    let start = if lo == 0 {
        0
    } else {
        entry(cx, table, lo - 1)?.0
    };
    // 10. Return CreateSegmentDataObject(segmenter, string, startIndex, endIndex).
    segment_data(cx, segmenter, string, start, end, word_like)
}

/// %IntlSegmentsPrototype% [ @@iterator ] ( ) (19.5.2.2).
fn segments_iterator<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    const C: ObjectClass = ObjectClass::IntlSegments;
    const I: ObjectClass = ObjectClass::IntlSegmentIterator;
    // 1-2. Let segments be the this value; ? RequireInternalSlot(segments,
    //      [[SegmentsSegmenter]]).
    // 3. Let segmenter be segments.[[SegmentsSegmenter]].
    let segmenter = cx.slot(this, C, segments_slot::SEGMENTER)?;
    // 4. Let string be segments.[[SegmentsString]].
    let string = cx.slot(this, C, segments_slot::STRING)?;
    // A table that `containing` made serves the iterator too (the same segmenter and string;
    // not observable).
    let table = cx.slot(this, C, segments_slot::TABLE)?;
    // 5. Return CreateSegmentIterator(segmenter, string) (19.6.1).
    let proto = cx.intrinsic(IntrinsicId::IntlSegmentIteratorPrototype);
    let it = cx.new_object_with_slots(proto, I, iterator_slot::COUNT);
    cx.set_slot(it, I, iterator_slot::SEGMENTER, segmenter)?;
    cx.set_slot(it, I, iterator_slot::STRING, string)?;
    cx.set_slot(it, I, iterator_slot::NEXT_INDEX, Local::int32(0))?;
    cx.set_slot(it, I, iterator_slot::TABLE, table)?;
    cx.set_slot(it, I, iterator_slot::POSITION, Local::int32(0))?;
    Ok(it.into())
}

/// CreateIteratorResultObject ( value, done ) (ECMA-262 §7.4.14).
fn iter_result<'s>(cx: &mut Cx<'s>, value: Local<'s>, done: bool) -> JsResult<Local<'s>> {
    let o = cx.new_object();
    cx.create_data_property_or_throw(o, "value", value)?;
    cx.create_data_property_or_throw(o, "done", Local::boolean(done))?;
    Ok(o.into())
}

/// %IntlSegmentIteratorPrototype%.next ( ) (19.6.2.1).
fn next<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    const I: ObjectClass = ObjectClass::IntlSegmentIterator;
    // 1-2. Let iterator be the this value; ? RequireInternalSlot(iterator,
    //      [[IteratingSegmenter]]).
    // 3. Let segmenter be iterator.[[IteratingSegmenter]].
    let segmenter = cx.slot(this, I, iterator_slot::SEGMENTER)?;
    // 4. Let string be iterator.[[IteratedString]].
    let string = string_of(cx.slot(this, I, iterator_slot::STRING)?);
    // 5. Let startIndex be iterator.[[IteratedStringNextSegmentCodeUnitIndex]].
    let start = cx
        .slot(this, I, iterator_slot::NEXT_INDEX)?
        .as_int32()
        .unwrap_or(0) as usize;
    // 6. Let len be the length of string.
    // 7. If startIndex ≥ len, return CreateIteratorResultObject(undefined, true).
    if start >= cx.string_len(string) {
        return iter_result(cx, Local::undefined(), true);
    }
    // 8. Let endIndex be FindBoundary(segmenter, string, startIndex, after): the end of the
    //    table entry at the iterator's position (the segment that starts at startIndex).
    let table = match cx.slot(this, I, iterator_slot::TABLE)?.as_object() {
        Some(t) => t,
        None => {
            let t = make_table(cx, segmenter, string)?;
            cx.set_slot(this, I, iterator_slot::TABLE, t)?;
            t
        }
    };
    let position = cx
        .slot(this, I, iterator_slot::POSITION)?
        .as_int32()
        .unwrap_or(0);
    let (end, word_like) = entry(cx, table, position as u64)?;
    // 9. Set iterator.[[IteratedStringNextSegmentCodeUnitIndex]] to endIndex.
    cx.set_slot(this, I, iterator_slot::NEXT_INDEX, Local::int32(end as i32))?;
    cx.set_slot(this, I, iterator_slot::POSITION, Local::int32(position + 1))?;
    // 10. Let segmentData be CreateSegmentDataObject(segmenter, string, startIndex, endIndex).
    let data = segment_data(cx, segmenter, string, start, end, word_like)?;
    // 11. Return CreateIteratorResultObject(segmentData, false).
    iter_result(cx, data, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16s(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// The (end, isWordLike) pairs of `s`.
    fn ends(locale: &str, g: Granularity, s: &str) -> Vec<(u32, bool)> {
        segment_ends(locale, g, &u16s(s), &mut |_| true)
            .expect("not stopped")
            .into_iter()
            .map(|e| (e >> 1, e & 1 != 0))
            .collect()
    }

    #[test]
    fn graphemes_words_and_sentences() {
        assert_eq!(ends("en", Granularity::Grapheme, ""), []);
        // e + U+0301, the flag of Japan (two regional indicators: 4 code units), "a".
        assert_eq!(
            ends("en", Granularity::Grapheme, "e\u{301}\u{1F1EF}\u{1F1F5}a"),
            [(2, false), (6, false), (7, false)]
        );
        assert_eq!(
            ends("en", Granularity::Word, "Hello, world 42!"),
            [
                (5, true),
                (6, false),
                (7, false),
                (12, true),
                (13, false),
                (15, true),
                (16, false)
            ]
        );
        assert_eq!(
            ends("en", Granularity::Sentence, "Hi there. How are you?"),
            [(10, false), (22, false)]
        );
        // "el" ends a sentence at ';' (the Greek question mark), "en" does not.
        assert_eq!(
            ends("el", Granularity::Sentence, "hello; world"),
            [(7, false), (12, false)]
        );
        assert_eq!(
            ends("en", Granularity::Sentence, "hello; world"),
            [(12, false)]
        );
    }

    /// Thai and Japanese words come from the dictionary (no LSTM).
    #[test]
    fn complex_scripts_use_the_dictionary() {
        assert_eq!(
            ends("th", Granularity::Word, "ทุกสองสัปดาห์"),
            [(3, true), (6, true), (13, true)]
        );
        assert_eq!(
            ends("ja", Granularity::Word, "こんにちは世界"),
            [(5, true), (7, true)]
        );
    }

    /// The scan polls after each boundary that ICU4X gives (the start, 0, first), with
    /// 1 + segment length / 256 units: a `false` from the third poll stops the scan of
    /// 200,000 code units after two words.
    #[test]
    fn the_scan_polls_per_boundary() {
        let text = u16s(&"a ".repeat(100_000));
        let mut calls = Vec::new();
        let r = segment_ends("en", Granularity::Word, &text, &mut |n| {
            calls.push(n);
            calls.len() < 3
        });
        assert_eq!((r, calls), (None, vec![1, 1, 1]));
        let long = u16s(&"a".repeat(1_000));
        let mut calls = Vec::new();
        let r = segment_ends("en", Granularity::Grapheme, &long, &mut |n| {
            calls.push(n);
            true
        });
        assert_eq!(r.map(|e| e.len()), Some(1_000));
        assert_eq!((calls.len(), calls.iter().sum::<usize>()), (1_001, 1_001));
        let mut calls = Vec::new();
        let r = segment_ends("en", Granularity::Word, &long, &mut |n| {
            calls.push(n);
            true
        });
        assert_eq!(
            (r.map(|e| e.len()), calls),
            (Some(1), vec![1, 1 + 1_000 / POLL_UNITS])
        );
    }
}
