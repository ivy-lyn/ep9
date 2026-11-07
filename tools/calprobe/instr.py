#!/usr/bin/env python3
"""Make a counting copy of a crate (calprobe, INTL-I1-D2): add `crate::__probe::hit(N);` at
the start of every non-const fn body under <crate>/src, and a `__probe` module at the end of
src/lib.rs (a counter per fn, `each`, `reset`). Site N is named "<file>:<line>:<fn>".
usage: python3 -I instr.py <crate dir>   (edits the copy in place; never a registry dir)"""
import os
import re
import sys

root = sys.argv[1]
src = os.path.join(root, "src")
if "registry" in os.path.abspath(root).split(os.sep):
    sys.exit("instr.py: refuses to edit a registry source; copy it first")
names = []
fn_re = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")
for dirpath, _, files in os.walk(src):
    for f in sorted(files):
        if not f.endswith(".rs"):
            continue
        p = os.path.join(dirpath, f)
        text = open(p, encoding="utf-8").read()
        out, i = [], 0
        while True:
            m = fn_re.search(text, i)
            if not m:
                out.append(text[i:])
                break
            line_start = text.rfind("\n", 0, m.start()) + 1
            prefix = text[line_start:m.start()]
            # Skip comments, const fns and macro text.
            if (prefix.lstrip().startswith("//")
                    or re.search(r"\bconst\s+(unsafe\s+)?$", prefix)
                    or "macro_rules" in prefix):
                out.append(text[i:m.end()])
                i = m.end()
                continue
            # The body starts at the first `{` outside () and []; a `;` first is a declaration.
            j, depth = m.end(), 0
            while j < len(text):
                c = text[j]
                if c in "([":
                    depth += 1
                elif c in ")]":
                    depth -= 1
                elif depth == 0 and c in "{;":
                    break
                j += 1
            if j >= len(text) or text[j] == ";":
                out.append(text[i:j + 1] if j < len(text) else text[i:])
                i = j + 1
                continue
            rel = os.path.relpath(p, src)
            line = text.count("\n", 0, m.start()) + 1
            out.append(text[i:j + 1])
            out.append(f" crate::__probe::hit({len(names)});")
            names.append(f"{rel}:{line}:{m.group(1)}")
            i = j + 1
        open(p, "w", encoding="utf-8").write("".join(out))
n = len(names)
mod = (
    "\n#[doc(hidden)]\n#[allow(missing_docs, missing_debug_implementations, unused_qualifications)]\n"
    "pub mod __probe {\n"
    "    use core::sync::atomic::{AtomicU64, Ordering};\n"
    f"    pub static HITS: [AtomicU64; {n}] = [const {{ AtomicU64::new(0) }}; {n}];\n"
    "    pub static NAMES: &[&str] = &[" + ",".join('"%s"' % s for s in names) + "];\n"
    "    #[inline(never)]\n"
    "    pub fn hit(i: usize) { HITS[i].fetch_add(1, Ordering::Relaxed); }\n"
    "    pub fn reset() { for h in HITS.iter() { h.store(0, Ordering::Relaxed); } }\n"
    "    pub fn each(f: &mut dyn FnMut(&'static str, u64)) {\n"
    "        for (i, h) in HITS.iter().enumerate() {\n"
    "            let v = h.load(Ordering::Relaxed);\n"
    "            if v != 0 { f(NAMES[i], v); }\n"
    "        }\n"
    "    }\n"
    "}\n"
)
open(os.path.join(src, "lib.rs"), "a", encoding="utf-8").write(mod)
print(f"instr.py: {os.path.basename(os.path.normpath(root))}: {n} sites")
