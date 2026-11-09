#!/usr/bin/env python3
"""Writes crates/ep9_rx1/src/unicode_tables.rs from Unicode Character Database files.

    python3 crates/ep9_rx1/data/gen-tables.py <ucd dir> > crates/ep9_rx1/src/unicode_tables.rs

<ucd dir> holds PropertyValueAliases.txt and DerivedCoreProperties.txt of one UCD version
(https://www.unicode.org/Public/<version>/ucd/). The version comes from the first line of each
file; both must be the same. The output is the same for the same files (sorted tables).

Tables (each sorted, for a binary search):
- GENERAL_CATEGORY: every General_Category value alias (gc lines) -> its short name.
- SCRIPT: every Script value alias (sc lines) -> its short name.
- ID_START, ID_CONTINUE: the code point ranges of the two derived properties, merged.
"""
import re
import sys


def version(path):
    with open(path, encoding="utf-8") as f:
        first = f.readline()
    m = re.match(r"# \S+-(\d+\.\d+\.\d+)\.txt", first)
    if not m:
        sys.exit(f"{path}: no version in the first line")
    return m.group(1)


def value_aliases(path, prop):
    rows = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            fields = [x.strip() for x in line.split(";")]
            if fields[0] != prop:
                continue
            short = fields[1]
            for alias in fields[1:]:
                rows.append((alias, short))
    rows = sorted(set(rows))
    names = [a for a, _ in rows]
    if len(names) != len(set(names)):
        sys.exit(f"{prop}: one alias names two values")
    return rows


def ranges(path, prop):
    out = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            cps, name = [x.strip() for x in line.split(";")[:2]]
            if name != prop:
                continue
            if ".." in cps:
                lo, hi = (int(x, 16) for x in cps.split(".."))
            else:
                lo = hi = int(cps, 16)
            out.append((lo, hi))
    out.sort()
    merged = []
    for lo, hi in out:
        if merged and lo <= merged[-1][1] + 1:
            merged[-1] = (merged[-1][0], max(merged[-1][1], hi))
        else:
            merged.append((lo, hi))
    return merged


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: gen-tables.py <ucd dir>")
    d = sys.argv[1]
    pva = f"{d}/PropertyValueAliases.txt"
    dcp = f"{d}/DerivedCoreProperties.txt"
    v = version(pva)
    if version(dcp) != v:
        sys.exit("the two files have different UCD versions")
    out = []
    w = out.append
    w("//! Unicode tables of the RX1 parser (generated; do not edit).")
    w("//!")
    w(f"//! Made by `crates/ep9_rx1/data/gen-tables.py` from the Unicode Character Database {v}")
    w(f"//! (<https://www.unicode.org/Public/{v}/ucd/>): `PropertyValueAliases.txt` (the gc and sc")
    w("//! lines) and `DerivedCoreProperties.txt` (ID_Start, ID_Continue). Unicode data, Unicode")
    w("//! License v3 (THIRD-PARTY-LICENSE). Each table is sorted for a binary search.")
    w("")
    w("/// The Unicode version of the tables.")
    w(f'pub const UCD_VERSION: &str = "{v}";')
    for name, prop, doc in (
        ("GENERAL_CATEGORY", "gc", "General_Category"),
        ("SCRIPT", "sc", "Script"),
    ):
        rows = value_aliases(pva, prop)
        w("")
        w(f"/// Every {doc} value alias and the short name of its value ({len(rows)} aliases).")
        w(f"pub(crate) static {name}: &[(&str, &str)] = &[")
        for alias, short in rows:
            w(f'    ("{alias}", "{short}"),')
        w("];")
    for name, prop in (("ID_START", "ID_Start"), ("ID_CONTINUE", "ID_Continue")):
        rs = ranges(dcp, prop)
        w("")
        w(f"/// The code points of {prop}, as inclusive ranges ({len(rs)} ranges).")
        w(f"pub(crate) static {name}: &[(u32, u32)] = &[")
        for lo, hi in rs:
            w(f"    (0x{lo:04X}, 0x{hi:04X}),")
        w("];")
    sys.stdout.write("\n".join(out) + "\n")


main()
