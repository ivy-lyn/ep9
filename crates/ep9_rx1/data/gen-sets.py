#!/usr/bin/env python3
"""Writes crates/ep9_rx1/src/unicode_sets.rs from Unicode Character Database files.

    python3 crates/ep9_rx1/data/gen-sets.py <ucd dir> > crates/ep9_rx1/src/unicode_sets.rs

These are the sets that V8 asks ICU for when it compiles a RegExp (RX1 port step 2): the code
points of each `\\p{..}` value, the strings of the properties of strings, and the case data of
`src/regexp/special-case.h`. The case part is a port of V8's
`src/regexp/gen-regexp-special-case.cc` (it computes IgnoreSet and SupplementaryEquivalentsSet
with the same steps and the same check), with ICU's UnicodeSet::closeOver
(USET_SIMPLE_CASE_INSENSITIVE) as the Simple_Case_Folding classes and ICU's toUpper as the
default full uppercase mapping.

<ucd dir> holds these files of one UCD version (https://www.unicode.org/Public/<version>/ucd/
and .../emoji/): UnicodeData.txt, PropertyValueAliases.txt, Scripts.txt, ScriptExtensions.txt,
PropList.txt, DerivedCoreProperties.txt, DerivedBinaryProperties.txt,
DerivedNormalizationProps.txt, CaseFolding.txt, SpecialCasing.txt, emoji-data.txt,
emoji-sequences.txt and emoji-zwj-sequences.txt. The UCD version comes from the first line of
each versioned file (UnicodeData.txt has none) and the emoji version from its "# Version:" line; all must be the same
major.minor. The output is the same for the same files (sorted tables, no time stamp); the
header names the sha256 of each file.
"""
import hashlib
import re
import sys
import textwrap

MAX = 0x10FFFF

UCD_FILES = [
    "UnicodeData.txt",
    "PropertyValueAliases.txt",
    "Scripts.txt",
    "ScriptExtensions.txt",
    "PropList.txt",
    "DerivedCoreProperties.txt",
    "DerivedBinaryProperties.txt",
    "DerivedNormalizationProps.txt",
    "CaseFolding.txt",
    "SpecialCasing.txt",
]
EMOJI_FILES = ["emoji-data.txt", "emoji-sequences.txt", "emoji-zwj-sequences.txt"]

# ECMA-262 table 68 (the binary properties) without Any, ASCII and Assigned (their sets are
# made by the compiler, as V8's LookupSpecialPropertyValueName does): canonical name -> file.
BINARY = {
    "ASCII_Hex_Digit": "PropList.txt",
    "Alphabetic": "DerivedCoreProperties.txt",
    "Bidi_Control": "PropList.txt",
    "Bidi_Mirrored": "DerivedBinaryProperties.txt",
    "Case_Ignorable": "DerivedCoreProperties.txt",
    "Cased": "DerivedCoreProperties.txt",
    "Changes_When_Casefolded": "DerivedCoreProperties.txt",
    "Changes_When_Casemapped": "DerivedCoreProperties.txt",
    "Changes_When_Lowercased": "DerivedCoreProperties.txt",
    "Changes_When_NFKC_Casefolded": "DerivedNormalizationProps.txt",
    "Changes_When_Titlecased": "DerivedCoreProperties.txt",
    "Changes_When_Uppercased": "DerivedCoreProperties.txt",
    "Dash": "PropList.txt",
    "Default_Ignorable_Code_Point": "DerivedCoreProperties.txt",
    "Deprecated": "PropList.txt",
    "Diacritic": "PropList.txt",
    "Emoji": "emoji-data.txt",
    "Emoji_Component": "emoji-data.txt",
    "Emoji_Modifier": "emoji-data.txt",
    "Emoji_Modifier_Base": "emoji-data.txt",
    "Emoji_Presentation": "emoji-data.txt",
    "Extended_Pictographic": "emoji-data.txt",
    "Extender": "PropList.txt",
    "Grapheme_Base": "DerivedCoreProperties.txt",
    "Grapheme_Extend": "DerivedCoreProperties.txt",
    "Hex_Digit": "PropList.txt",
    "IDS_Binary_Operator": "PropList.txt",
    "IDS_Trinary_Operator": "PropList.txt",
    "ID_Continue": "DerivedCoreProperties.txt",
    "ID_Start": "DerivedCoreProperties.txt",
    "Ideographic": "PropList.txt",
    "Join_Control": "PropList.txt",
    "Logical_Order_Exception": "PropList.txt",
    "Lowercase": "DerivedCoreProperties.txt",
    "Math": "DerivedCoreProperties.txt",
    "Noncharacter_Code_Point": "PropList.txt",
    "Pattern_Syntax": "PropList.txt",
    "Pattern_White_Space": "PropList.txt",
    "Quotation_Mark": "PropList.txt",
    "Radical": "PropList.txt",
    "Regional_Indicator": "PropList.txt",
    "Sentence_Terminal": "PropList.txt",
    "Soft_Dotted": "PropList.txt",
    "Terminal_Punctuation": "PropList.txt",
    "Unified_Ideograph": "PropList.txt",
    "Uppercase": "DerivedCoreProperties.txt",
    "Variation_Selector": "PropList.txt",
    "White_Space": "PropList.txt",
    "XID_Continue": "DerivedCoreProperties.txt",
    "XID_Start": "DerivedCoreProperties.txt",
}

# ECMA-262 table 69 (the binary properties of strings) without RGI_Emoji, the union of the six
# (the compiler makes it): name -> file.
STRINGS = {
    "Basic_Emoji": "emoji-sequences.txt",
    "Emoji_Keycap_Sequence": "emoji-sequences.txt",
    "RGI_Emoji_Flag_Sequence": "emoji-sequences.txt",
    "RGI_Emoji_Modifier_Sequence": "emoji-sequences.txt",
    "RGI_Emoji_Tag_Sequence": "emoji-sequences.txt",
    "RGI_Emoji_ZWJ_Sequence": "emoji-zwj-sequences.txt",
}

# The General_Category groups (UAX #44 table 12, "Values of the General_Category").
GC_GROUPS = {
    "C": ["Cc", "Cf", "Cs", "Co", "Cn"],
    "L": ["Lu", "Ll", "Lt", "Lm", "Lo"],
    "LC": ["Lu", "Ll", "Lt"],
    "M": ["Mn", "Mc", "Me"],
    "N": ["Nd", "Nl", "No"],
    "P": ["Pc", "Pd", "Ps", "Pe", "Pi", "Pf", "Po"],
    "S": ["Sm", "Sc", "Sk", "So"],
    "Z": ["Zs", "Zl", "Zp"],
}


def die(msg):
    sys.exit(f"gen-sets.py: {msg}")


def ucd_version(path):
    with open(path, encoding="utf-8") as f:
        first = f.readline()
    m = re.match(r"# \S+-(\d+\.\d+\.\d+)\.txt", first)
    return m.group(1) if m else None


def emoji_version(path):
    with open(path, encoding="utf-8") as f:
        for line in f:
            m = re.match(r"# Version: (\d+\.\d+)", line)
            if m:
                return m.group(1)
    return None


def rows(path):
    """The data lines of a UCD file: fields split at ';', comment and spaces removed."""
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if line:
                yield [x.strip() for x in line.split(";")]


def cp_range(text):
    if ".." in text:
        lo, hi = (int(x, 16) for x in text.split(".."))
    else:
        lo = hi = int(text, 16)
    return lo, hi


def merge(ranges):
    out = []
    for lo, hi in sorted(ranges):
        if out and lo <= out[-1][1] + 1:
            out[-1] = (out[-1][0], max(out[-1][1], hi))
        else:
            out.append((lo, hi))
    return out


def ranges_of(points):
    return merge((c, c) for c in points)


def binary_ranges(path, prop):
    out = []
    for f in rows(path):
        if len(f) == 2 and f[1] == prop:
            out.append(cp_range(f[0]))
    if not out:
        die(f"{prop}: no lines in {path}")
    return merge(out)


def general_categories(path):
    """Code point -> gc short name for every code point (Cn where UnicodeData has none)."""
    gc = ["Cn"] * (MAX + 1)
    first = None
    for f in rows(path):
        cp = int(f[0], 16)
        name, cat = f[1], f[2]
        if name.endswith(", First>"):
            first = cp
            continue
        if name.endswith(", Last>"):
            for c in range(first, cp + 1):
                gc[c] = cat
            first = None
            continue
        gc[cp] = cat
    return gc


def value_aliases(path, prop):
    """Every value of `prop` in PropertyValueAliases.txt: short name -> its other names."""
    out = {}
    for f in rows(path):
        if f[0] == prop:
            out[f[1]] = f[2:]
    return out


def scripts(path, sc_names):
    """Code point -> sc short name (Zzzz where Scripts.txt has none)."""
    long_to_short = {}
    for short, others in sc_names.items():
        for name in others:
            long_to_short[name] = short
        long_to_short[short] = short
    sc = ["Zzzz"] * (MAX + 1)
    for f in rows(path):
        lo, hi = cp_range(f[0])
        short = long_to_short.get(f[1])
        if short is None:
            die(f"Scripts.txt: unknown script {f[1]}")
        for c in range(lo, hi + 1):
            sc[c] = short
    return sc


def script_extensions(path, sc, sc_names):
    """Code point -> the set of sc short names of its Script_Extensions value."""
    listed = {}
    for f in rows(path):
        lo, hi = cp_range(f[0])
        names = f[1].split()
        for n in names:
            if n not in sc_names:
                die(f"ScriptExtensions.txt: unknown script {n}")
        for c in range(lo, hi + 1):
            listed[c] = set(names)
    return listed


def simple_folding(path):
    """Code point -> scf (CaseFolding.txt status C and S), for the code points it changes."""
    scf = {}
    for f in rows(path):
        if f[1] in ("C", "S"):
            scf[int(f[0], 16)] = int(f[2], 16)
    return scf


def uppercase(ud_path, sc_path):
    """Code point -> its default full uppercase mapping (a tuple of code points), where it is
    not the code point itself: UnicodeData field 12, then the unconditional SpecialCasing.txt
    lines (they win)."""
    upper = {}
    for f in rows(ud_path):
        if f[12]:
            upper[int(f[0], 16)] = (int(f[12], 16),)
    for f in rows(sc_path):
        # code; lower; title; upper; (condition_list;)
        if len(f) >= 5 and f[4]:
            continue
        upper[int(f[0], 16)] = tuple(int(x, 16) for x in f[3].split())
    return upper


def utf16_len(cps):
    return sum(2 if c > 0xFFFF else 1 for c in cps)


def canonicalize(ch, upper):
    """ECMA-262 22.2.2.7.3 Canonicalize, rer.[[Unicode]] false (V8 gen-regexp-special-case.cc
    Canonicalize): ch is a UTF-16 code unit."""
    u = upper.get(ch, (ch,))
    if utf16_len(u) != 1:
        return ch
    cu = u[0]
    if ch >= 128 and cu < 128:
        return ch
    return cu


def closure_classes(scf):
    """The Simple_Case_Folding classes with more than one member (ICU's closeOver with
    USET_SIMPLE_CASE_INSENSITIVE): folded value -> sorted members."""
    by_fold = {}
    for c, f in scf.items():
        by_fold.setdefault(f, {f}).add(c)
    return {f: sorted(m) for f, m in by_fold.items() if len(m) > 1}


def close_over(c, cls_of):
    m = cls_of.get(c)
    return set(m) if m else {c}


def special_case(scf, upper):
    """Port of V8 gen-regexp-special-case.cc PrintSpecial: IgnoreSet and
    SupplementaryEquivalentsSet, with V8's check."""
    classes = closure_classes(scf)
    cls_of = {}
    for members in classes.values():
        for c in members:
            cls_of[c] = members
    special_add = set()
    ignore = set()
    supplementary = set()
    for i in range(0x10000):
        if 0xD800 <= i <= 0xDFFF:
            continue
        current = close_over(i, cls_of)
        if any(c >= 0x10000 for c in current):
            supplementary.add(i)
            current = {c for c in current if c < 0x10000}
        canonical = canonicalize(i, upper)
        matching = False
        non_matching = False
        for c in current:
            if c == i:
                continue
            if canonicalize(c, upper) == canonical:
                matching = True
            else:
                non_matching = True
        if non_matching:
            if matching:
                special_add.add(i)
            else:
                ignore.add(i)
    # V8's check: no Unicode class holds two non-trivial JS classes.
    for c in sorted(special_add):
        canonical = canonicalize(c, upper)
        current = {x for x in close_over(c, cls_of) if x < 0x10000} - ignore
        for c2 in current:
            if canonicalize(c2, upper) != canonical:
                die(f"special case check fails at U+{c:04X} / U+{c2:04X}")
    # And the claim of special-case.h: CloseOver(kNonUnicode) gives the ECMA-262 classes.
    canon_class = {}
    for c in range(0x10000):
        canon_class.setdefault(canonicalize(c, upper), set()).add(c)
    for c in range(0x10000):
        if 0xD800 <= c <= 0xDFFF:
            continue
        want = canon_class[canonicalize(c, upper)]
        if c in ignore:
            got = {c}
        else:
            got = {x for x in close_over(c, cls_of) if x < 0x10000} - ignore
        if got != want:
            die(f"CloseOver(kNonUnicode) differs from Canonicalize at U+{c:04X}")
    return classes, ignore, supplementary


def emoji_sequences(path):
    """Type name -> (single code points, strings of two or more code points)."""
    out = {}
    for f in rows(path):
        cps, kind = f[0], f[1]
        singles, strings = out.setdefault(kind, (set(), set()))
        parts = cps.split()
        if len(parts) == 1:
            lo, hi = cp_range(parts[0])
            singles.update(range(lo, hi + 1))
        else:
            if any(".." in p for p in parts):
                die(f"{path}: a range inside a sequence: {cps}")
            strings.add(tuple(int(p, 16) for p in parts))
    return out


def fmt_ranges(rs, indent):
    """Ranges as Rust tuples, several to a line (the module is rustfmt::skip)."""
    items = [f"(0x{lo:04X}, 0x{hi:04X})," for lo, hi in rs]
    lines = []
    line = indent
    for it in items:
        if len(line) + len(it) + 1 > 99 and line.strip():
            lines.append(line.rstrip())
            line = indent
        line += it + " "
    if line.strip():
        lines.append(line.rstrip())
    return lines


def main():
    if len(sys.argv) != 2:
        die("usage: gen-sets.py <ucd dir>")
    d = sys.argv[1]
    # UnicodeData.txt has no version line; its sha256 in the header names it.
    versions = {ucd_version(f"{d}/{n}") for n in UCD_FILES if n != "UnicodeData.txt"}
    if len(versions) != 1 or None in versions:
        die(f"the UCD files have different or no versions: {versions}")
    v = versions.pop()
    ev = {emoji_version(f"{d}/{n}") for n in EMOJI_FILES}
    if ev != {".".join(v.split(".")[:2])}:
        die(f"the emoji files are not version {v}: {ev}")
    sha = {}
    for n in UCD_FILES + EMOJI_FILES:
        with open(f"{d}/{n}", "rb") as f:
            sha[n] = hashlib.sha256(f.read()).hexdigest()

    gc_of = general_categories(f"{d}/UnicodeData.txt")
    gc_names = value_aliases(f"{d}/PropertyValueAliases.txt", "gc")
    gc_sets = {}
    for c, cat in enumerate(gc_of):
        gc_sets.setdefault(cat, []).append((c, c))
    gc_sets = {k: merge(rs) for k, rs in gc_sets.items()}
    for group, leaves in GC_GROUPS.items():
        gc_sets[group] = merge(r for leaf in leaves for r in gc_sets.get(leaf, []))
    if set(gc_sets) != set(gc_names):
        die(f"gc values differ: {sorted(set(gc_sets) ^ set(gc_names))}")

    sc_names = value_aliases(f"{d}/PropertyValueAliases.txt", "sc")
    sc_of = scripts(f"{d}/Scripts.txt", sc_names)
    scx_listed = script_extensions(f"{d}/ScriptExtensions.txt", sc_of, sc_names)
    sc_sets = {s: [] for s in sc_names}
    scx_sets = {s: [] for s in sc_names}
    for c, s in enumerate(sc_of):
        sc_sets[s].append((c, c))
        for x in scx_listed.get(c, {s}):
            scx_sets[x].append((c, c))
    sc_sets = {k: merge(rs) for k, rs in sc_sets.items()}
    scx_sets = {k: merge(rs) for k, rs in scx_sets.items()}

    binary = {name: binary_ranges(f"{d}/{path}", name) for name, path in BINARY.items()}

    seqs = emoji_sequences(f"{d}/emoji-sequences.txt")
    for kind, val in emoji_sequences(f"{d}/emoji-zwj-sequences.txt").items():
        seqs[kind] = val
    if set(seqs) != set(STRINGS):
        die(f"the emoji sequence types differ: {sorted(set(seqs) ^ set(STRINGS))}")

    scf = simple_folding(f"{d}/CaseFolding.txt")
    upper = uppercase(f"{d}/UnicodeData.txt", f"{d}/SpecialCasing.txt")
    classes, ignore, supplementary = special_case(scf, upper)
    orbit = []
    for members in classes.values():
        for k, c in enumerate(members):
            orbit.append((c, members[(k + 1) % len(members)]))
    orbit.sort()

    out = []
    w = out.append
    w("//! The code point sets and the case data of the RX1 compiler (generated; do not edit).")
    w("//!")
    w(f"//! Made by `crates/ep9_rx1/data/gen-sets.py` from the Unicode Character Database {v}")
    w(f"//! (<https://www.unicode.org/Public/{v}/ucd/>, emoji data {'.'.join(v.split('.')[:2])}).")
    w("//! Unicode data, Unicode License v3 (THIRD-PARTY-LICENSE). The input files (sha256):")
    w("//!")
    w("//! ```text")
    for n in UCD_FILES + EMOJI_FILES:
        w(f"//! {n:<30} {sha[n]}")
    w("//! ```")
    w("//!")
    w("//! Every range list is sorted, with no two ranges that overlap or touch (V8's canonical")
    w("//! form, `CharacterRange::IsCanonical`). Each named table is sorted by name.")
    w("")
    w("/// The Unicode version of the sets (a test checks that it is the parser's version).")
    w("#[cfg(test)]")
    w(f'pub(crate) const VERSION: &str = "{v}";')
    w("")
    w("/// A range list: inclusive `(from, to)` code point pairs.")
    w("pub(crate) type Ranges = &'static [(u32, u32)];")

    def named(name, doc, table):
        w("")
        for line in textwrap.wrap(f"{doc} ({len(table)} values).", 95):
            w(f"/// {line}")
        w(f"pub(crate) static {name}: &[(&str, Ranges)] = &[")
        for key in sorted(table):
            rs = table[key]
            if not rs:
                w(f'    ("{key}", &[]),')
                continue
            w(f'    ("{key}", &[')
            out.extend(fmt_ranges(rs, "        "))
            w("    ]),")
        w("];")

    named(
        "GENERAL_CATEGORY",
        "The code points of each General_Category value, by its short name (the groups too)",
        gc_sets,
    )
    named("SCRIPT", "The code points of each Script value, by its short name", sc_sets)
    named(
        "SCRIPT_EXTENSIONS",
        "The code points of each Script_Extensions value, by its Script short name",
        scx_sets,
    )
    named(
        "BINARY",
        "The code points of each binary property of ECMA-262 table 68 but Any, ASCII and"
        " Assigned, by its canonical name",
        binary,
    )

    w("")
    w("/// A property of strings: its single code points and its longer strings.")
    w("pub(crate) struct StringProperty {")
    w("    /// The canonical name (ECMA-262 table 69).")
    w("    pub(crate) name: &'static str,")
    w("    /// The strings of one code point, as ranges.")
    w("    pub(crate) singles: Ranges,")
    w("    /// The strings of two or more code points, sorted.")
    w("    pub(crate) strings: &'static [&'static [u32]],")
    w("}")
    w("")
    w(f"/// The properties of strings of ECMA-262 table 69 but RGI_Emoji ({len(STRINGS)} values).")
    w("pub(crate) static STRING_PROPERTIES: &[StringProperty] = &[")
    for name in sorted(STRINGS):
        singles, strings = seqs[name]
        w("    StringProperty {")
        w(f'        name: "{name}",')
        rs = ranges_of(singles)
        if rs:
            w("        singles: &[")
            out.extend(fmt_ranges(rs, "            "))
            w("        ],")
        else:
            w("        singles: &[],")
        w("        strings: &[")
        for s in sorted(strings):
            w("            &[" + ", ".join(f"0x{c:04X}" for c in s) + "],")
        w("        ],")
        w("    },")
    w("];")

    w("")
    w("/// Simple_Case_Folding (CaseFolding.txt status C and S; ICU `u_foldCase` with")
    w(f"/// U_FOLD_CASE_DEFAULT): `(c, scf(c))` for each c that it changes ({len(scf)} pairs).")
    w("pub(crate) static SIMPLE_FOLDING: &[(u32, u32)] = &[")
    items = [f"(0x{c:04X}, 0x{f:04X})," for c, f in sorted(scf.items())]
    line = "   "
    for it in items:
        if len(line) + len(it) + 1 > 99:
            w(line.rstrip())
            line = "   "
        line += " " + it
    w(line.rstrip())
    w("];")
    w("")
    w("/// The Simple_Case_Folding classes of two or more members (ICU `closeOver` with")
    w("/// USET_SIMPLE_CASE_INSENSITIVE) as cycles: `(c, the next member of c's class)` for each")
    w(f"/// member, sorted by c ({len(orbit)} pairs in {len(classes)} classes).")
    w("pub(crate) static CASE_ORBIT: &[(u32, u32)] = &[")
    items = [f"(0x{c:04X}, 0x{n:04X})," for c, n in orbit]
    line = "   "
    for it in items:
        if len(line) + len(it) + 1 > 99:
            w(line.rstrip())
            line = "   "
        line += " " + it
    w(line.rstrip())
    w("];")
    w("")
    w("/// V8 `CaseFolding::IgnoreSet` (gen-regexp-special-case.cc): the BMP characters that")
    w("/// match only themselves under `i` without `u`, though their Simple_Case_Folding class")
    w("/// has more members.")
    w("pub(crate) static IGNORE_SET: Ranges = &[")
    out.extend(fmt_ranges(ranges_of(ignore), "    "))
    w("];")
    w("")
    w("/// V8 `CaseFolding::SupplementaryEquivalentsSet`: the BMP characters whose Simple_Case_")
    w("/// Folding class holds a code point above U+FFFF.")
    w("pub(crate) static SUPPLEMENTARY_EQUIVALENTS: Ranges = &[")
    rs = ranges_of(supplementary)
    if rs:
        out.extend(fmt_ranges(rs, "    "))
    w("];")
    sys.stdout.write("\n".join(out) + "\n")


main()
