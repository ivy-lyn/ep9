//! Intl.ListFormat (ECMA-402 §13; block 35): `%Intl.ListFormat%` and `%Intl.ListFormat.prototype%`,
//! a module of the lazy group "Intl" (INTL-I1-D8): its properties target only `%Intl%` and
//! its own intrinsics. Also the case mapping of `String.prototype.toLocaleLowerCase` and
//! `toLocaleUpperCase` ([`casemap`], ECMA-402 §19.1.2-3), the other half of the LIST+CASEMAP
//! service.
//!
//! ```text
//! Intl.ListFormat {w, c}       %Intl.ListFormat%  prototype {}, supportedLocalesOf
//! Intl.ListFormat.prototype    %Intl.ListFormat.prototype%  constructor {w, c},
//!                                format, formatToParts, resolvedOptions {w, c},
//!                                [@@toStringTag] "Intl.ListFormat" {c}
//! slots (ObjectClass::IntlListFormat, SLOTS = 2):
//!   0 [[Locale]]  a String
//!   1 options     an int32: bits 0-1 [[Type]] (index in TYPES), bits 2-3 [[Style]] (STYLES)
//! format(list) ─ StringListFromIterable ─ code units of each element (no Local kept)
//!   ─ cache::get_or_make(Service::ListFormat, data locale, options) ─> Rc<Joiner>
//!   ─ Joiner::parts ─> [(element | literal, code units)] ─ one String, or the parts Array
//! ```
//!
//! ICU4X gives the list text only as a whole (`ListFormatter::format_to_string`; the parts
//! API needs `writeable`, which is not an `ep9_intl` dependency). The CLDR list patterns have
//! no text before `{0}` (icu_list datagen rejects such a pattern), so a list of n elements is
//! `e0 start e1 (middle ei)* end e(n-1) after` (n >= 3) or `e0 pair e1 after` (n = 2). The
//! [`Joiner`] reads `start` and `middle` once from a list of private-use marks; the `end` and
//! `pair` literals can depend on the last element (Spanish "y" / "e"), so each format reads
//! them from a list of marks and the real last element. The output is built from those
//! literals and the elements' own code units, so a lone surrogate stays as it is. No ICU4X
//! object is in a slot (INTL-I1-D7).
//!
//! Ids are append-only: definition `i` of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by
//! `crates/ep9_builtins/tests/golden/ids-list_format.txt`.

pub mod casemap;

use ep9_vm::{
    Args, Atom, BuiltinDef, BuiltinModule, ConstValue, Cx, IntrinsicDef, IntrinsicId,
    IntrinsicValue, JsResult, Local, ObjectClass, ObjectProps, PropAttrs, PropDef, PropKey,
    WellKnownSymbol,
};
use icu_list::options::{ListFormatterOptions, ListLength};
use icu_list::{ListFormatter, ListFormatterPreferences};
use icu_locale_core::Locale;

use crate::cache::{self, Service};
use crate::options::{get_locale_matcher, get_options_object, get_string_option};
use crate::resolve;
use crate::tag;

/// This module's block of builtin ids.
pub const BLOCK: u8 = 35;

/// The class of an Intl.ListFormat object.
pub const CLASS: ObjectClass = ObjectClass::IntlListFormat;

/// Slot 0: `[[Locale]]`, the resolved locale (a String).
pub const SLOT_LOCALE: usize = 0;
/// Slot 1: the [`Packed`] `[[Type]]` and `[[Style]]` (an int32).
pub const SLOT_OPTIONS: usize = 1;
/// The slot count of an Intl.ListFormat object.
pub const SLOTS: usize = 2;

/// Definition indices.
mod def {
    pub const SUPPORTED_LOCALES_OF: u16 = 1;
    pub const FORMAT: u16 = 2;
    pub const FORMAT_TO_PARTS: u16 = 3;
    pub const RESOLVED_OPTIONS: u16 = 4;
}

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[
    BuiltinDef::constructor("ListFormat", 0, constructor), // 0 (13.1.1)
    BuiltinDef::method("supportedLocalesOf", 1, supported_locales_of), // 1 (13.2.2)
    BuiltinDef::method("format", 1, format),               // 2 (13.3.3)
    BuiltinDef::method("formatToParts", 1, format_to_parts), // 3 (13.3.4)
    BuiltinDef::method("resolvedOptions", 0, resolved_options), // 4 (13.3.5)
];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[
    IntrinsicDef {
        id: IntrinsicId::IntlListFormat,
        value: IntrinsicValue::Builtin(0),
    },
    IntrinsicDef {
        id: IntrinsicId::IntlListFormatPrototype,
        value: IntrinsicValue::Object {
            proto: Some(IntrinsicId::ObjectPrototype),
        },
    },
];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[
    ObjectProps {
        target: IntrinsicId::IntlListFormat,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Prototype),
                IntrinsicId::IntlListFormatPrototype,
                PropAttrs::NONE,
            ),
            PropDef::method(
                PropKey::Str("supportedLocalesOf"),
                def::SUPPORTED_LOCALES_OF,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::IntlListFormatPrototype,
        props: &[
            PropDef::intrinsic(
                PropKey::Atom(Atom::Constructor),
                IntrinsicId::IntlListFormat,
                PropAttrs::METHOD,
            ),
            // The key order of V8 (node): resolvedOptions, format, formatToParts.
            PropDef::method(PropKey::Str("resolvedOptions"), def::RESOLVED_OPTIONS),
            PropDef::method(PropKey::Str("format"), def::FORMAT),
            PropDef::method(PropKey::Str("formatToParts"), def::FORMAT_TO_PARTS),
            // Intl.ListFormat.prototype [ @@toStringTag ] ({c}, 13.3.2).
            PropDef::constant(
                PropKey::Symbol(WellKnownSymbol::TO_STRING_TAG),
                ConstValue::Str("Intl.ListFormat"),
                PropAttrs::CONFIGURABLE,
            ),
        ],
    },
    ObjectProps {
        target: IntrinsicId::Intl,
        props: &[PropDef::intrinsic(
            PropKey::Str("ListFormat"),
            IntrinsicId::IntlListFormat,
            PropAttrs::METHOD,
        )],
    },
];

/// The module.
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "list_format",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};

// ---- the resolved options ---------------------------------------------------------------------

/// `[[Type]]` values, in their packed order.
pub const TYPES: [&str; 3] = ["conjunction", "disjunction", "unit"];
/// `[[Style]]` values, in their packed order.
pub const STYLES: [&str; 3] = ["long", "short", "narrow"];

/// `[[Type]]` and `[[Style]]` of a list format, packed in one int32 (slot 1; also the cache
/// key): bits 0-1 the index in [`TYPES`], bits 2-3 the index in [`STYLES`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packed(pub u32);

impl Packed {
    /// Packs the index `ty` in [`TYPES`] and `style` in [`STYLES`].
    pub fn new(ty: usize, style: usize) -> Packed {
        Packed((ty as u32 & 3) | (style as u32 & 3) << 2)
    }

    /// `[[Type]]`.
    pub fn ty(self) -> &'static str {
        TYPES
            .get((self.0 & 3) as usize)
            .copied()
            .unwrap_or(TYPES[0])
    }

    /// `[[Style]]`.
    pub fn style(self) -> &'static str {
        STYLES
            .get((self.0 >> 2 & 3) as usize)
            .copied()
            .unwrap_or(STYLES[0])
    }
}

/// The result of the constructor's steps 3-15: `[[Locale]]` and the packed options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// `[[Locale]]`.
    pub locale: String,
    /// `[[Type]]` and `[[Style]]`.
    pub options: Packed,
}

/// Intl.ListFormat steps 3-15 (13.1.1; the object creation of step 2 is the caller's): the
/// `[[Locale]]` and the packed options for `locales` and `options`, in the spec order of the
/// observable steps. No ICU4X object is made.
pub fn initialize<'s>(
    cx: &mut Cx<'s>,
    locales: Local<'s>,
    options: Local<'s>,
) -> JsResult<Resolved> {
    // 3. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, locales)?;
    // 4. Set options to ? GetOptionsObject(options).
    let options = get_options_object(cx, options)?;
    // 5-7. Let matcher be ? GetOption(options, "localeMatcher", string, « "lookup",
    //      "best fit" », "best fit").
    get_locale_matcher(cx, options)?;
    // 8-9. Let r be ResolveLocale(%Intl.ListFormat%.[[AvailableLocales]], requestedLocales,
    //      opt, « », localeData); set listFormat.[[Locale]] to r.[[Locale]].
    let r = resolve::resolve_locale(cx, &requested, &[], &[], &|_, _| Vec::new())?;
    // 10-11. Let type be ? GetOption(options, "type", string, « "conjunction",
    //        "disjunction", "unit" », "conjunction").
    let ty = get_string_option(cx, options, "type", &TYPES, Some(TYPES[0]))?;
    // 12-13. Let style be ? GetOption(options, "style", string, « "long", "short",
    //        "narrow" », "long").
    let style = get_string_option(cx, options, "style", &STYLES, Some(STYLES[0]))?;
    let index = |list: &[&str], v: Option<String>| {
        list.iter()
            .position(|x| Some(*x) == v.as_deref())
            .unwrap_or(0)
    };
    // 14-15. [[Templates]] come from the data locale at the first format (the cache).
    Ok(Resolved {
        locale: r.locale,
        options: Packed::new(index(&TYPES, ty), index(&STYLES, style)),
    })
}

// ---- the ICU4X list formatter and the parts -----------------------------------------------------

/// The private-use marks that stand for the elements whose text does not change a literal
/// (no CLDR list pattern has a private-use character).
const MARKS: [&str; 4] = ["\u{E000}", "\u{E001}", "\u{E002}", "\u{E003}"];

/// The ICU4X list formatter of one data locale, type and style, with the literals that do not
/// depend on an element.
pub struct Joiner {
    formatter: ListFormatter,
    /// The literal after the first element of a list of 3 or more.
    start: String,
    /// The literal between two middle elements.
    middle: String,
    /// The `end` literal after the last element for a last element of private-use text (the
    /// usual `after` of the locale; it picks the split of [`split_last`]).
    end_after: String,
    /// The `pair` literal after the second element, as `end_after`.
    pair_after: String,
}

impl Joiner {
    /// A new joiner for `data_locale` (canonical, no extension) and `options`; `None` if the
    /// ICU4X data has no list patterns for it (not the case with the compiled data, which
    /// falls back to the root patterns).
    pub fn new(data_locale: &str, options: Packed) -> Option<Joiner> {
        let locale = Locale::try_from_str(data_locale).unwrap_or(Locale::UNKNOWN);
        let prefs = ListFormatterPreferences::from(&locale);
        let length = match options.style() {
            "short" => ListLength::Short,
            "narrow" => ListLength::Narrow,
            _ => ListLength::Wide,
        };
        let o = ListFormatterOptions::default().with_length(length);
        let formatter = match options.ty() {
            "disjunction" => ListFormatter::try_new_or(prefs, o),
            "unit" => ListFormatter::try_new_unit(prefs, o),
            _ => ListFormatter::try_new_and(prefs, o),
        }
        .ok()?;
        // e0 start e1 middle e2 end e3 after.
        let four = formatter.format_to_string(MARKS.iter().copied());
        let start = between(&four, MARKS[0], MARKS[1])?;
        let middle = between(&four, MARKS[1], MARKS[2])?;
        let end_after = four.split_once(MARKS[3])?.1.to_string();
        // e0 pair e1 after.
        let two = formatter.format_to_string(MARKS[..2].iter().copied());
        let pair_after = two.split_once(MARKS[1])?.1.to_string();
        Some(Joiner {
            formatter,
            start: start.to_string(),
            middle: middle.to_string(),
            end_after,
            pair_after,
        })
    }

    /// The `end` (n >= 3) or `pair` (n = 2) literals before and after `last`, the text of the
    /// last element: ICU4X formats marks and `last`, and the text after the marks is
    /// `before last after`.
    fn last_literals(&self, n: usize, last: &str) -> (String, String) {
        let (marks, prefix_len, after) = if n == 2 {
            (&MARKS[..1], MARKS[0].len(), &self.pair_after)
        } else {
            (
                &MARKS[..2],
                MARKS[0].len() + self.start.len() + MARKS[1].len(),
                &self.end_after,
            )
        };
        let text = self
            .formatter
            .format_to_string(marks.iter().copied().chain(core::iter::once(last)));
        let rest = text.get(prefix_len..).unwrap_or("");
        split_last(rest, last, after)
    }

    /// The parts of the list `elements` (CreatePartsFromList, 13.5.2 steps 1-4, the
    /// `[[Templates]]` of the data): element and literal parts, in order, as code units. An
    /// empty literal is not a part.
    pub fn parts(&self, elements: &[Vec<u16>]) -> Vec<(PartKind, Vec<u16>)> {
        let n = elements.len();
        let mut out: Vec<(PartKind, Vec<u16>)> = Vec::with_capacity(2 * n + 1);
        let literal = |out: &mut Vec<(PartKind, Vec<u16>)>, s: &str| {
            if !s.is_empty() {
                out.push((PartKind::Literal, s.encode_utf16().collect()));
            }
        };
        match n {
            0 => {}
            1 => out.push((PartKind::Element, elements[0].clone())),
            _ => {
                let last = String::from_utf16_lossy(&elements[n - 1]);
                let (before, after) = self.last_literals(n, &last);
                for (i, e) in elements.iter().enumerate() {
                    if i == n - 1 {
                        literal(&mut out, &before);
                    } else if i == 1 {
                        literal(&mut out, &self.start);
                    } else if i > 1 {
                        literal(&mut out, &self.middle);
                    }
                    out.push((PartKind::Element, e.clone()));
                }
                literal(&mut out, &after);
            }
        }
        out
    }
}

/// The text of `s` between the marks `a` and `b`.
fn between<'t>(s: &'t str, a: &str, b: &str) -> Option<&'t str> {
    let (_, rest) = s.split_once(a)?;
    Some(rest.split_once(b)?.0)
}

/// Splits `rest` = `before last after` into (`before`, `after`). The `after` of the locale
/// (`usual_after`, almost always "") picks the split when `last` is found there; else the last
/// place of `last` in `rest` does (a literal ends before the last element).
fn split_last(rest: &str, last: &str, usual_after: &str) -> (String, String) {
    if let Some(head) = rest.strip_suffix(usual_after)
        && let Some(before) = head.strip_suffix(last)
    {
        return (before.to_string(), usual_after.to_string());
    }
    match rest.rfind(last) {
        Some(i) => (rest[..i].to_string(), rest[i + last.len()..].to_string()),
        None => (rest.to_string(), String::new()),
    }
}

/// The type of a list part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    /// An element of the list.
    Element,
    /// A literal of the pattern.
    Literal,
}

impl PartKind {
    /// The `type` of the part object.
    pub fn name(self) -> &'static str {
        match self {
            PartKind::Element => "element",
            PartKind::Literal => "literal",
        }
    }
}

/// The cached joiner of `locale` (a `[[Locale]]`) and `options` (crate::cache; the "und"
/// patterns if the locale has none).
fn joiner_for(locale: &str, options: Packed) -> Option<std::rc::Rc<Joiner>> {
    let data_locale = tag::remove_unicode_extension(locale);
    cache::get_or_make(
        Service::ListFormat,
        &data_locale,
        u64::from(options.0),
        || Joiner::new(&data_locale, options).or_else(|| Joiner::new("und", options)),
    )
}

/// CreatePartsFromList ( listFormat, list ) (13.5.2) for the `[[Locale]]` `locale` and the
/// packed options: the parts of `list` (the code units of each element).
pub fn create_parts_from_list(
    locale: &str,
    options: Packed,
    list: &[Vec<u16>],
) -> Vec<(PartKind, Vec<u16>)> {
    match joiner_for(locale, options) {
        Some(j) => j.parts(list),
        // No data at all (not the case with the compiled data): the elements, joined by ", ".
        None => {
            let mut out = Vec::new();
            for (i, e) in list.iter().enumerate() {
                if i > 0 {
                    out.push((PartKind::Literal, ", ".encode_utf16().collect()));
                }
                out.push((PartKind::Element, e.clone()));
            }
            out
        }
    }
}

// ---- the builtins -----------------------------------------------------------------------------

/// Intl.ListFormat ( [ locales [ , options ] ] ) (13.1.1).
fn constructor<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. If NewTarget is undefined, throw a TypeError exception.
    let Some(nt) = new_target.as_object() else {
        return Err(cx.throw_type_error("Constructor Intl.ListFormat requires 'new'"));
    };
    // 2. Let listFormat be ? OrdinaryCreateFromConstructor(NewTarget,
    //    "%Intl.ListFormat.prototype%", « [[InitializedListFormat]], [[Locale]], [[Type]],
    //    [[Style]], [[Templates]] »).
    let proto = cx.get_prototype_from_constructor(nt, IntrinsicId::IntlListFormatPrototype)?;
    let lf = cx.new_object_with_slots(proto, CLASS, SLOTS);
    // 3-15.
    let r = initialize(cx, args.get(0), args.get(1))?;
    let locale = cx.new_string(&r.locale)?;
    cx.set_slot(lf, CLASS, SLOT_LOCALE, locale)?;
    cx.set_slot(lf, CLASS, SLOT_OPTIONS, Local::int32(r.options.0 as i32))?;
    // 16. Return listFormat.
    Ok(lf.into())
}

/// Intl.ListFormat.supportedLocalesOf ( locales [ , options ] ) (13.2.2).
fn supported_locales_of<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1. Let availableLocales be %Intl.ListFormat%.[[AvailableLocales]].
    // 2. Let requestedLocales be ? CanonicalizeLocaleList(locales).
    let requested = tag::canonicalize_locale_list(cx, args.get(0))?;
    // 3. Return ? FilterLocales(availableLocales, requestedLocales, options).
    resolve::supported_locales(cx, &requested, args.get(1))
}

/// The `[[Locale]]` text and the packed options of the list format `o` (TypeError if `o` is
/// not an Intl.ListFormat: RequireInternalSlot(lf, `[[InitializedListFormat]]`)).
fn read<'s>(cx: &mut Cx<'s>, o: Local<'s>) -> JsResult<(String, Packed)> {
    let locale = cx.slot(o, CLASS, SLOT_LOCALE)?;
    let options = cx.slot(o, CLASS, SLOT_OPTIONS)?;
    let locale = cx.to_rust_string(locale)?;
    Ok((locale, Packed(options.as_int32().unwrap_or(0) as u32)))
}

/// The code units of the string `s`, copied out (a view cannot live across a GC point).
fn units<'s>(cx: &mut Cx<'s>, s: Local<'s, ep9_vm::JsString>) -> JsResult<Vec<u16>> {
    let len = cx.string_len(s);
    let flat = cx.flatten(s)?;
    Ok((0..len)
        .map(|i| cx.code_unit_at(flat, i).unwrap_or(0))
        .collect())
}

/// StringListFromIterable ( iterable ) (13.5.3): the code units of each String the iterable
/// gives. A value that is not a String closes the iterator with a TypeError.
pub fn string_list_from_iterable<'s>(
    cx: &mut Cx<'s>,
    iterable: Local<'s>,
) -> JsResult<Vec<Vec<u16>>> {
    // 1. If iterable is undefined, return a new empty List.
    if iterable.is_undefined() {
        return Ok(Vec::new());
    }
    // 2. Let iteratorRecord be ? GetIterator(iterable, sync).
    let mut rec = cx.get_iterator(iterable)?;
    // 3. Let list be a new empty List.
    let mut list: Vec<Vec<u16>> = Vec::new();
    // 4. Repeat,
    //    a-b. Let next be ? IteratorStepValue(iteratorRecord); if next is done, return list.
    while let Some(next) = cx.iterator_step_value(&mut rec)? {
        // c. If next is not a String, then
        let Some(s) = next.as_string() else {
            // i. Let error be ThrowCompletion(a newly created TypeError object).
            let error = Err(cx.throw_type_error("Iterable yielded a value that is not a string"));
            // ii. Return ? IteratorClose(iteratorRecord, error).
            return cx.iterator_close(&rec, error);
        };
        // d. Append next to list.
        let u = units(cx, s)?;
        list.push(u);
    }
    Ok(list)
}

/// The steps 1-3 of format and formatToParts: the list format of `this` and the string list.
fn read_list<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    list: Local<'s>,
) -> JsResult<Vec<(PartKind, Vec<u16>)>> {
    // 1-2. Let lf be the this value; ? RequireInternalSlot(lf, [[InitializedListFormat]]).
    let (locale, options) = read(cx, this)?;
    // 3. Let stringList be ? StringListFromIterable(list).
    let strings = string_list_from_iterable(cx, list)?;
    Ok(create_parts_from_list(&locale, options, &strings))
}

/// Intl.ListFormat.prototype.format ( list ) (13.3.3).
fn format<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let parts = read_list(cx, this, args.get(0))?;
    // 4. Return FormatList(lf, stringList) (13.5.4): the concatenation of the parts.
    let len: usize = parts.iter().map(|(_, u)| u.len()).sum();
    let mut out: Vec<u16> = Vec::with_capacity(len);
    for (_, u) in &parts {
        out.extend_from_slice(u);
    }
    Ok(cx.new_string_utf16(&out)?.into())
}

/// Intl.ListFormat.prototype.formatToParts ( list ) (13.3.4).
fn format_to_parts<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let parts = read_list(cx, this, args.get(0))?;
    // 4. Return FormatListToParts(lf, stringList) (13.5.5): an Array of { type, value }.
    let array = cx.new_array();
    for (i, (kind, value)) in parts.iter().enumerate() {
        let o = cx.new_object();
        let t = cx.new_string(kind.name())?;
        cx.create_data_property_or_throw(o, "type", t)?;
        let v = cx.new_string_utf16(value)?;
        cx.create_data_property_or_throw(o, "value", v)?;
        cx.create_data_property_index_or_throw(array, i as u64, o)?;
    }
    Ok(array.into())
}

/// Intl.ListFormat.prototype.resolvedOptions ( ) (13.3.5).
fn resolved_options<'s>(
    cx: &mut Cx<'s>,
    this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    // 1-2. Let lf be the this value; ? RequireInternalSlot(lf, [[InitializedListFormat]]).
    let (locale, p) = read(cx, this)?;
    // 3. Let options be OrdinaryObjectCreate(%Object.prototype%).
    let o = cx.new_object();
    // 4. For each row of Table 25 (locale, type, style), in table order:
    //    CreateDataPropertyOrThrow(options, p, v).
    for (key, value) in [
        ("locale", locale.as_str()),
        ("type", p.ty()),
        ("style", p.style()),
    ] {
        let v = cx.new_string(value)?;
        cx.create_data_property_or_throw(o, key, v)?;
    }
    // 5. Return options.
    Ok(o.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn text(parts: &[(PartKind, Vec<u16>)]) -> String {
        parts
            .iter()
            .map(|(k, v)| {
                let v = String::from_utf16_lossy(v);
                match k {
                    PartKind::Element => format!("[{v}]"),
                    PartKind::Literal => v,
                }
            })
            .collect()
    }

    fn fmt(locale: &str, ty: usize, style: usize, list: &[&str]) -> String {
        let list: Vec<Vec<u16>> = list.iter().map(|s| u(s)).collect();
        text(&create_parts_from_list(
            locale,
            Packed::new(ty, style),
            &list,
        ))
    }

    #[test]
    fn packed_round_trips() {
        for (t, ty) in TYPES.iter().enumerate() {
            for (s, style) in STYLES.iter().enumerate() {
                let p = Packed::new(t, s);
                assert_eq!((p.ty(), p.style()), (*ty, *style));
            }
        }
    }

    #[test]
    fn english_and_spanish_parts() {
        assert_eq!(fmt("en", 0, 0, &[]), "");
        assert_eq!(fmt("en", 0, 0, &["a"]), "[a]");
        assert_eq!(fmt("en", 0, 0, &["a", "b"]), "[a] and [b]");
        assert_eq!(
            fmt("en", 0, 0, &["a", "b", "c", "d"]),
            "[a], [b], [c], and [d]"
        );
        assert_eq!(fmt("en", 1, 1, &["a", "b", "c"]), "[a], [b], or [c]");
        assert_eq!(fmt("en", 2, 2, &["a", "b", "c"]), "[a] [b] [c]");
        // The Spanish "y" becomes "e" before an "i" sound; "o" becomes "u" before an "o".
        assert_eq!(
            fmt("es", 0, 0, &["a", "b", "iglesia"]),
            "[a], [b] e [iglesia]"
        );
        assert_eq!(fmt("es", 0, 0, &["a", "hielo"]), "[a] y [hielo]");
        assert_eq!(fmt("es", 1, 0, &["siete", "ocho"]), "[siete] u [ocho]");
    }

    #[test]
    fn an_element_equal_to_a_literal_keeps_its_place() {
        assert_eq!(fmt("en", 0, 0, &["and", "and"]), "[and] and [and]");
        assert_eq!(fmt("en", 0, 0, &[" and ", ""]), "[ and ] and []");
        assert_eq!(fmt("en", 0, 0, &["", "", ""]), "[], [], and []");
        assert_eq!(fmt("en", 2, 0, &["a", ", "]), "[a], [, ]");
    }

    /// For every available locale, type and style, the parts joined are the ICU4X text of the
    /// same list, for lists whose last element picks a conditional literal (Spanish "e" and
    /// "u", Hebrew "ו-", ...), and lists whose elements hold literal text.
    #[test]
    fn the_parts_join_to_the_icu4x_text_in_every_locale() {
        let lists: &[&[&str]] = &[
            &["a", "b"],
            &["a", "b", "c"],
            &["a", "b", "c", "d", "e"],
            &["x", "iglesia"],
            &["x", "y", "hielo"],
            &["x", "ocho"],
            &["x", "y", "8"],
            &["x", "11"],
            &["\u{5d0}", "A"],
            &["\u{5d0}", "\u{5d1}", "1"],
            &["x", "and"],
            &[", ", ", ", ", "],
            &["", ""],
            &["a b", "c d", ""],
        ];
        let (mut checked, mut diffs) = (0usize, Vec::new());
        for locale in crate::available::locales() {
            for t in 0..3 {
                for st in 0..3 {
                    let p = Packed::new(t, st);
                    let j = Joiner::new(locale, p).expect("list data");
                    for list in lists {
                        let units: Vec<Vec<u16>> = list.iter().map(|s| u(s)).collect();
                        let got: String = j
                            .parts(&units)
                            .iter()
                            .map(|(_, v)| String::from_utf16_lossy(v))
                            .collect();
                        let want = j.formatter.format_to_string(list.iter().copied());
                        if got != want {
                            diffs.push(format!("{locale} {p:?} {list:?}: {got:?} != {want:?}"));
                        }
                        checked += 1;
                    }
                }
            }
        }
        eprintln!("{checked} lists, {} differences", diffs.len());
        assert!(checked > 10_000);
        assert!(diffs.is_empty(), "{:?}", &diffs[..diffs.len().min(10)]);
    }

    #[test]
    fn split_last_prefers_the_usual_after() {
        assert_eq!(
            split_last(" and x and x", "x and x", ""),
            (" and ".to_string(), String::new())
        );
        assert_eq!(
            split_last("、xです", "x", "です"),
            ("、".to_string(), "です".to_string())
        );
        assert_eq!(
            split_last(" y z.", "z", ""),
            (" y ".to_string(), ".".to_string())
        );
    }
}
