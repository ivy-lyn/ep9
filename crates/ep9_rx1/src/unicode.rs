//! The Unicode lookups of the parser: property names (V8 `AddPropertyClassRange`, which asks
//! ICU) and the identifier classes of group names (V8 `IsIdentifierStart`, `IsIdentifierPart`).
//!
//! ```text
//! \p{name}         gc value alias?  -> GeneralCategory
//!                  Any / ASCII / Assigned
//!                  binary property (ECMA-262 table 68)?          -> Binary
//!                  property of strings (table 69; v only, no \P)? -> OfStrings
//! \p{name=value}   name gc / General_Category  + gc value alias  -> GeneralCategory
//!                  name sc / Script            + sc value alias  -> Script
//!                  name scx / Script_Extensions + sc value alias -> ScriptExtensions
//! ```
//!
//! The names follow ECMA-262 22.2.2.9 (tables 67-69 and PropertyValueAliases.txt), with no
//! loose matching. Two V8 rules are not ported, because V8 takes them from ICU and the spec
//! does not have them (ECMA-262: "implementations must not support any other property names
//! or aliases"): V8 also takes the ICU short names of binary properties that the spec table
//! does not list (for example `\p{WSpace}`), and V8 refuses a value whose set is empty
//! (`\p{sc=Hrkt}`). The value tables are Unicode [`UCD_VERSION`](crate::UCD_VERSION).

use crate::ast::Property;
use crate::unicode_tables::{GENERAL_CATEGORY, ID_CONTINUE, ID_START, SCRIPT};

/// ECMA-262 table 68: the binary properties, each name and alias -> the canonical name.
/// `Any`, `ASCII` and `Assigned` are in the table too, but they have their own
/// [`Property`] values.
static BINARY: &[(&str, &str)] = &[
    ("AHex", "ASCII_Hex_Digit"),
    ("ASCII_Hex_Digit", "ASCII_Hex_Digit"),
    ("Alpha", "Alphabetic"),
    ("Alphabetic", "Alphabetic"),
    ("Bidi_C", "Bidi_Control"),
    ("Bidi_Control", "Bidi_Control"),
    ("Bidi_M", "Bidi_Mirrored"),
    ("Bidi_Mirrored", "Bidi_Mirrored"),
    ("CI", "Case_Ignorable"),
    ("CWCF", "Changes_When_Casefolded"),
    ("CWCM", "Changes_When_Casemapped"),
    ("CWKCF", "Changes_When_NFKC_Casefolded"),
    ("CWL", "Changes_When_Lowercased"),
    ("CWT", "Changes_When_Titlecased"),
    ("CWU", "Changes_When_Uppercased"),
    ("Case_Ignorable", "Case_Ignorable"),
    ("Cased", "Cased"),
    ("Changes_When_Casefolded", "Changes_When_Casefolded"),
    ("Changes_When_Casemapped", "Changes_When_Casemapped"),
    ("Changes_When_Lowercased", "Changes_When_Lowercased"),
    (
        "Changes_When_NFKC_Casefolded",
        "Changes_When_NFKC_Casefolded",
    ),
    ("Changes_When_Titlecased", "Changes_When_Titlecased"),
    ("Changes_When_Uppercased", "Changes_When_Uppercased"),
    ("DI", "Default_Ignorable_Code_Point"),
    ("Dash", "Dash"),
    (
        "Default_Ignorable_Code_Point",
        "Default_Ignorable_Code_Point",
    ),
    ("Dep", "Deprecated"),
    ("Deprecated", "Deprecated"),
    ("Dia", "Diacritic"),
    ("Diacritic", "Diacritic"),
    ("EBase", "Emoji_Modifier_Base"),
    ("EComp", "Emoji_Component"),
    ("EMod", "Emoji_Modifier"),
    ("EPres", "Emoji_Presentation"),
    ("Emoji", "Emoji"),
    ("Emoji_Component", "Emoji_Component"),
    ("Emoji_Modifier", "Emoji_Modifier"),
    ("Emoji_Modifier_Base", "Emoji_Modifier_Base"),
    ("Emoji_Presentation", "Emoji_Presentation"),
    ("Ext", "Extender"),
    ("ExtPict", "Extended_Pictographic"),
    ("Extended_Pictographic", "Extended_Pictographic"),
    ("Extender", "Extender"),
    ("Gr_Base", "Grapheme_Base"),
    ("Gr_Ext", "Grapheme_Extend"),
    ("Grapheme_Base", "Grapheme_Base"),
    ("Grapheme_Extend", "Grapheme_Extend"),
    ("Hex", "Hex_Digit"),
    ("Hex_Digit", "Hex_Digit"),
    ("IDC", "ID_Continue"),
    ("IDS", "ID_Start"),
    ("IDSB", "IDS_Binary_Operator"),
    ("IDST", "IDS_Trinary_Operator"),
    ("IDS_Binary_Operator", "IDS_Binary_Operator"),
    ("IDS_Trinary_Operator", "IDS_Trinary_Operator"),
    ("ID_Continue", "ID_Continue"),
    ("ID_Start", "ID_Start"),
    ("Ideo", "Ideographic"),
    ("Ideographic", "Ideographic"),
    ("Join_C", "Join_Control"),
    ("Join_Control", "Join_Control"),
    ("LOE", "Logical_Order_Exception"),
    ("Logical_Order_Exception", "Logical_Order_Exception"),
    ("Lower", "Lowercase"),
    ("Lowercase", "Lowercase"),
    ("Math", "Math"),
    ("NChar", "Noncharacter_Code_Point"),
    ("Noncharacter_Code_Point", "Noncharacter_Code_Point"),
    ("Pat_Syn", "Pattern_Syntax"),
    ("Pat_WS", "Pattern_White_Space"),
    ("Pattern_Syntax", "Pattern_Syntax"),
    ("Pattern_White_Space", "Pattern_White_Space"),
    ("QMark", "Quotation_Mark"),
    ("Quotation_Mark", "Quotation_Mark"),
    ("RI", "Regional_Indicator"),
    ("Radical", "Radical"),
    ("Regional_Indicator", "Regional_Indicator"),
    ("SD", "Soft_Dotted"),
    ("STerm", "Sentence_Terminal"),
    ("Sentence_Terminal", "Sentence_Terminal"),
    ("Soft_Dotted", "Soft_Dotted"),
    ("Term", "Terminal_Punctuation"),
    ("Terminal_Punctuation", "Terminal_Punctuation"),
    ("UIdeo", "Unified_Ideograph"),
    ("Unified_Ideograph", "Unified_Ideograph"),
    ("Upper", "Uppercase"),
    ("Uppercase", "Uppercase"),
    ("VS", "Variation_Selector"),
    ("Variation_Selector", "Variation_Selector"),
    ("White_Space", "White_Space"),
    ("XIDC", "XID_Continue"),
    ("XIDS", "XID_Start"),
    ("XID_Continue", "XID_Continue"),
    ("XID_Start", "XID_Start"),
    ("space", "White_Space"),
];

/// ECMA-262 table 69: the binary properties of strings (`v` only; no aliases).
static OF_STRINGS: &[&str] = &[
    "Basic_Emoji",
    "Emoji_Keycap_Sequence",
    "RGI_Emoji",
    "RGI_Emoji_Flag_Sequence",
    "RGI_Emoji_Modifier_Sequence",
    "RGI_Emoji_Tag_Sequence",
    "RGI_Emoji_ZWJ_Sequence",
];

/// Test only: every name and alias of a binary property (table 68 without Any, ASCII and
/// Assigned).
#[cfg(test)]
pub(crate) fn binary_names() -> impl Iterator<Item = &'static str> {
    BINARY.iter().map(|&(name, _)| name)
}

fn lookup(table: &'static [(&'static str, &'static str)], name: &str) -> Option<&'static str> {
    table
        .binary_search_by(|(alias, _)| (*alias).cmp(name))
        .ok()
        .map(|i| table[i].1)
}

/// The property of `\p{name_1}` (no `=`), or `None` (V8 `AddPropertyClassRange`, the
/// `name_2.empty()` arm). The caller checks the `\P` rule of properties of strings.
pub(crate) fn lone_property(name: &str, unicode_sets: bool) -> Option<Property> {
    if let Some(gc) = lookup(GENERAL_CATEGORY, name) {
        return Some(Property::GeneralCategory(gc));
    }
    match name {
        "Any" => return Some(Property::Any),
        "ASCII" => return Some(Property::Ascii),
        "Assigned" => return Some(Property::Assigned),
        _ => {}
    }
    if let Some(binary) = lookup(BINARY, name) {
        return Some(Property::Binary(binary));
    }
    if unicode_sets && let Ok(i) = OF_STRINGS.binary_search(&name) {
        return Some(Property::OfStrings(OF_STRINGS[i]));
    }
    None
}

/// The property of `\p{name=value}`, or `None` (V8 `AddPropertyClassRange`, the second arm).
pub(crate) fn property_with_value(name: &str, value: &str) -> Option<Property> {
    match name {
        "General_Category" | "gc" => lookup(GENERAL_CATEGORY, value).map(Property::GeneralCategory),
        "Script" | "sc" => lookup(SCRIPT, value).map(Property::Script),
        "Script_Extensions" | "scx" => lookup(SCRIPT, value).map(Property::ScriptExtensions),
        _ => None,
    }
}

fn in_ranges(table: &[(u32, u32)], c: u32) -> bool {
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < c {
                std::cmp::Ordering::Less
            } else if lo > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// RegExpIdentifierStart without the escape forms: ID_Start, `$`, `_` (V8
/// `IsIdentifierStart`; V8 also takes `\`, which the parser refuses first).
pub(crate) fn is_identifier_start(c: u32) -> bool {
    c == u32::from(b'$') || c == u32::from(b'_') || in_ranges(ID_START, c)
}

/// RegExpIdentifierPart without the escape forms: ID_Continue, `$`, ZWNJ, ZWJ (V8
/// `IsIdentifierPart`; `_` is in ID_Continue).
pub(crate) fn is_identifier_part(c: u32) -> bool {
    c == u32::from(b'$') || c == 0x200c || c == 0x200d || in_ranges(ID_CONTINUE, c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(names: impl Iterator<Item = &'static str>) -> bool {
        let v: Vec<&str> = names.collect();
        v.windows(2).all(|w| w[0] < w[1])
    }

    #[test]
    fn tables_are_sorted_for_the_binary_search() {
        assert!(sorted(BINARY.iter().map(|r| r.0)));
        assert!(sorted(OF_STRINGS.iter().copied()));
        assert!(sorted(GENERAL_CATEGORY.iter().map(|r| r.0)));
        assert!(sorted(SCRIPT.iter().map(|r| r.0)));
        for t in [ID_START, ID_CONTINUE] {
            assert!(t.windows(2).all(|w| w[0].1 + 1 < w[1].0));
            assert!(t.iter().all(|r| r.0 <= r.1));
        }
    }

    #[test]
    fn binary_table_has_the_53_spec_properties() {
        // Table 68 has 53 rows: Any, ASCII and Assigned (their own Property values) and 50
        // here.
        let mut canonical: Vec<&str> = BINARY.iter().map(|r| r.1).collect();
        canonical.dedup();
        canonical.sort_unstable();
        canonical.dedup();
        assert_eq!(canonical.len(), 50);
        // Each canonical name maps to itself.
        for c in canonical {
            assert_eq!(lookup(BINARY, c), Some(c));
        }
    }

    #[test]
    fn lookups_follow_the_spec_tables() {
        assert_eq!(
            lone_property("Lu", false),
            Some(Property::GeneralCategory("Lu"))
        );
        assert_eq!(
            lone_property("Letter", false),
            Some(Property::GeneralCategory("L"))
        );
        assert_eq!(
            lone_property("digit", false),
            Some(Property::GeneralCategory("Nd"))
        );
        assert_eq!(
            lone_property("space", false),
            Some(Property::Binary("White_Space"))
        );
        assert_eq!(lone_property("WSpace", false), None);
        assert_eq!(lone_property("Any", false), Some(Property::Any));
        assert_eq!(lone_property("RGI_Emoji", false), None);
        assert_eq!(
            lone_property("RGI_Emoji", true),
            Some(Property::OfStrings("RGI_Emoji"))
        );
        assert_eq!(lone_property("lu", false), None);
        assert_eq!(
            property_with_value("sc", "Grek"),
            Some(Property::Script("Grek"))
        );
        assert_eq!(
            property_with_value("Script", "Greek"),
            Some(Property::Script("Grek"))
        );
        assert_eq!(
            property_with_value("scx", "Qaai"),
            Some(Property::ScriptExtensions("Zinh"))
        );
        assert_eq!(property_with_value("gc", "Any"), None);
        assert_eq!(property_with_value("ASCII", "Y"), None);
        assert_eq!(
            property_with_value("sc", "Berf"),
            Some(Property::Script("Berf"))
        );
    }

    #[test]
    fn identifier_classes() {
        for c in ['a', 'Z', '$', '_', 'π', '\u{10400}'] {
            assert!(is_identifier_start(u32::from(c)), "{c:?}");
        }
        for c in ['1', '-', '\u{200c}', '\\'] {
            assert!(!is_identifier_start(u32::from(c)), "{c:?}");
        }
        for c in ['1', '_', '\u{200c}', '\u{200d}', '\u{0301}'] {
            assert!(is_identifier_part(u32::from(c)), "{c:?}");
        }
        assert!(!is_identifier_part(u32::from('-')));
        assert!(!is_identifier_part(0xd800));
    }
}
