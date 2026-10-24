//! The H20 core golden (docs/adr/H20-builtin-abi.md, "Freeze table"; M3-P3 FREEZE-PREP task
//! 5): `tests/golden/h20-core.txt` lists every public name and signature of the builtin ABI,
//! read from the source:
//!
//! ```text
//! src/abi/*.rs     every pub item: types (fields, variants, derives), traits, the pub items
//!                  of their impl blocks (`Cx`, `Args`, `Local`, ...), trait impls, the marker
//!                  macro lists, the `pub use` re-exports
//! src/isolate.rs   the `impl Cx` block (the scoped embedder forms), and the embedder entry
//!                  points of `impl Isolate` (`ENTRY_POINTS`: the calls that run ABI code or
//!                  install a hook that ABI code calls; H20-Q4)
//! src/builtins.rs  the builtin table types and `check_tables`
//! src/lib.rs       the crate-root re-exports of `abi` and `builtins` (the public paths), and
//!                  the path of `Isolate`
//! ```
//!
//! A line is a signature without parameter names (a rename is not an API change), with the
//! attributes that matter to a caller (`derive`, `repr`, `non_exhaustive`, `cfg`,
//! `doc(hidden)`, `deprecated`). Comments, doc text and bodies are not read. Not here: the
//! append-only lists that have their own goldens (the `IntrinsicId` and `ObjectClass` variants,
//! `tests/golden/vm-builtin-ids.txt`, `tests/golden/object-classes.txt`), private and
//! `pub(crate)` items, and `export_facts` (H20 part 2, V8-A13: only its path is here).
//!
//! The core freezes at M0 exit. After that a line that changes or goes away is a change to a
//! Frozen part (README §3.2); a new line is an addition to the part-2 `Cx` method list (append
//! only). The test tells the two apart. `EP9_BLESS=1 cargo test -p ep9_vm --test h20_core`
//! rewrites the file; review the diff: after the freeze, only added lines are allowed.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/h20-core.txt")
}

/// The source files the golden reads, as (path under `src/`, text), `abi/` sorted by name.
fn sources() -> Vec<(String, String)> {
    let mut abi: Vec<String> = std::fs::read_dir(src_dir().join("abi"))
        .expect("src/abi")
        .map(|e| e.expect("a dir entry").file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".rs"))
        .map(|n| format!("abi/{n}"))
        .collect();
    abi.sort();
    abi.extend(["isolate.rs", "builtins.rs", "lib.rs"].map(String::from));
    abi.into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(src_dir().join(&p))
                .unwrap_or_else(|e| panic!("src/{p}: {e}"));
            (p, text.replace("\r\n", "\n"))
        })
        .collect()
}

// ---- source text without comments and literals ---------------------------------------------

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A char of a string literal as `strip` keeps it.
fn mask(c: char) -> char {
    match c {
        '{' | '}' | '[' | ']' | '(' | ')' | ';' | '/' | '\'' | '\n' => '_',
        c => c,
    }
}

/// `src` with comments removed (each becomes a space), char literals as `' '`, and in string,
/// raw string and byte string literals every bracket, `;` and `/` as `_` (an attribute such as
/// `#[cfg(feature = "x")]` keeps its text), so the brackets and `;` left are code. Lifetimes
/// (`'s`) stay.
fn strip(src: &str) -> String {
    let c: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < c.len() {
        let prev_ident = i > 0 && is_ident(c[i - 1]);
        if c[i] == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            out.push(' ');
        } else if c[i] == '/' && c.get(i + 1) == Some(&'*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            out.push(' ');
        } else if !prev_ident && (c[i] == 'r' || (c[i] == 'b' && c.get(i + 1) == Some(&'r'))) && {
            let mut j = i + if c[i] == 'b' { 2 } else { 1 };
            while c.get(j) == Some(&'#') {
                j += 1;
            }
            c.get(j) == Some(&'"')
        } {
            // A raw string: r#"..."# with the same number of #.
            let mut j = i + if c[i] == 'b' { 2 } else { 1 };
            let mut hashes = 0;
            while c[j] == '#' {
                hashes += 1;
                j += 1;
            }
            j += 1;
            let start = j;
            loop {
                if c[j] == '"' && (0..hashes).all(|k| c.get(j + 1 + k) == Some(&'#')) {
                    break;
                }
                j += 1;
            }
            out.push('"');
            out.extend(c[start..j].iter().map(|&x| mask(x)));
            out.push('"');
            i = j + 1 + hashes;
        } else if c[i] == '"' || (!prev_ident && c[i] == 'b' && c.get(i + 1) == Some(&'"')) {
            let start = i + if c[i] == 'b' { 2 } else { 1 };
            let mut j = start;
            while c[j] != '"' {
                j += if c[j] == '\\' { 2 } else { 1 };
            }
            out.push('"');
            out.extend(c[start..j].iter().map(|&x| mask(x)));
            out.push('"');
            i = j + 1;
        } else if c[i] == '\'' || (!prev_ident && c[i] == 'b' && c.get(i + 1) == Some(&'\'')) {
            let q = if c[i] == 'b' { i + 1 } else { i };
            if c.get(q + 1) == Some(&'\\') {
                // An escaped char literal: '\n', '\'', '\u{..}'.
                let mut j = q + 3;
                while c[j] != '\'' {
                    j += 1;
                }
                out.push_str("' '");
                i = j + 1;
            } else if c.get(q + 2) == Some(&'\'') {
                out.push_str("' '");
                i = q + 3;
            } else {
                // A lifetime or a label.
                out.push('\'');
                i += 1;
            }
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

// ---- items ----------------------------------------------------------------------------------

/// One item of a stripped text: its outer attributes, its head (the text before its body or
/// its `;`) and its `{}` body, if it has one.
#[derive(Debug)]
struct Item {
    attrs: Vec<String>,
    head: String,
    body: Option<String>,
}

/// The index after the bracket that closes the one at `open` (`{`, `[` or `(`).
fn close(c: &[char], open: usize) -> usize {
    let mut depth = 0i32;
    let mut i = open;
    loop {
        match c[i] {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// The items at the top level of `text` (a stripped file or a stripped body).
fn items(text: &str) -> Vec<Item> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut attrs = Vec::new();
    let mut i = 0;
    while i < c.len() {
        if c[i].is_whitespace() || c[i] == ';' {
            i += 1;
        } else if c[i] == '#' && c.get(i + 1) == Some(&'!') {
            i = close(&c, i + 2);
        } else if c[i] == '#' && c.get(i + 1) == Some(&'[') {
            let end = close(&c, i + 1);
            attrs.push(norm(&c[i..end].iter().collect::<String>()));
            i = end;
        } else {
            let mut j = i;
            let mut depth = 0i32;
            while j < c.len() {
                match c[j] {
                    '(' | '[' => depth += 1,
                    ')' | ']' => depth -= 1,
                    '{' | ';' if depth == 0 => break,
                    _ => {}
                }
                j += 1;
            }
            let head = c[i..j.min(c.len())].iter().collect::<String>();
            let (body, next) = if j < c.len() && c[j] == '{' {
                let end = close(&c, j);
                (Some(c[j + 1..end - 1].iter().collect()), end)
            } else {
                (None, j + 1)
            };
            out.push(Item {
                attrs: std::mem::take(&mut attrs),
                head: norm(&head),
                body,
            });
            i = next;
        }
    }
    out
}

/// Whitespace runs to one space; no space inside `()` and `[]`, after the `<` of generics
/// or before `,`; no trailing comma; `{}` for an empty brace pair.
fn norm(s: &str) -> String {
    let mut out = String::new();
    for w in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    for (from, to) in [
        ("( ", "("),
        (" )", ")"),
        ("[ ", "["),
        (" ]", "]"),
        (" ,", ","),
        (",)", ")"),
        (",]", "]"),
        (",>", ">"),
        (",}", "}"),
        (", )", ")"),
        (", ]", "]"),
        (", >", ">"),
        (", }", " }"),
        ("{ }", "{}"),
    ] {
        while out.contains(from) {
            out = out.replace(from, to);
        }
    }
    // `Foo< A` (a generic list broken over lines) -> `Foo<A`; `1 << 3` stays.
    let c: Vec<char> = out.chars().collect();
    let mut s = String::with_capacity(out.len());
    for i in 0..c.len() {
        if c[i] == ' ' && i >= 2 && c[i - 1] == '<' && is_ident(c[i - 2]) {
            continue;
        }
        s.push(c[i]);
    }
    s
}

/// True if the head declares a `fn` (`pub const fn` too), not a `const` whose type or value
/// has a `fn` or `=`.
fn is_fn_head(head: &str) -> bool {
    let end = head.find(['(', '<', ':', '=']).unwrap_or(head.len());
    head[..end].split(' ').any(|w| w == "fn")
}

/// Splits `s` at the commas outside brackets (`()`, `[]`, `<>`, `{}`; the `>` of `->` and
/// `=>` closes nothing). Empty parts are dropped.
fn split_top(s: &str) -> Vec<String> {
    let c: Vec<char> = s.chars().collect();
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for i in 0..c.len() {
        match c[i] {
            '(' | '[' | '<' | '{' => depth += 1,
            '>' if i > 0 && (c[i - 1] == '-' || c[i - 1] == '=') => {}
            ')' | ']' | '>' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(c[start..i].iter().collect::<String>());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(c[start..].iter().collect());
    parts
        .into_iter()
        .map(|p| norm(&p))
        .filter(|p| !p.is_empty())
        .collect()
}

/// The attributes a caller depends on, joined with a trailing space ("" if none).
fn kept_attrs(attrs: &[String]) -> String {
    let mut s = String::new();
    for a in attrs {
        let name = a.trim_start_matches("#[");
        if [
            "derive",
            "repr",
            "non_exhaustive",
            "cfg",
            "doc(hidden)",
            "deprecated",
        ]
        .iter()
        .any(|k| name.starts_with(k))
        {
            s.push_str(a);
            s.push(' ');
        }
    }
    s
}

/// Leading outer attributes of a field, variant or parameter text, split off.
fn split_attrs(s: &str) -> (Vec<String>, String) {
    let c: Vec<char> = s.chars().collect();
    let mut attrs = Vec::new();
    let mut i = 0;
    loop {
        while i < c.len() && c[i].is_whitespace() {
            i += 1;
        }
        if c.get(i) == Some(&'#') && c.get(i + 1) == Some(&'[') {
            let end = close(&c, i + 1);
            attrs.push(norm(&c[i..end].iter().collect::<String>()));
            i = end;
        } else {
            break;
        }
    }
    (attrs, norm(&c[i..].iter().collect::<String>()))
}

/// The index of the first `:` at bracket depth 0 that is not part of `::`.
fn type_colon(p: &str) -> Option<usize> {
    let b = p.as_bytes();
    let mut depth = 0i32;
    for i in 0..b.len() {
        match b[i] {
            b'(' | b'[' | b'<' => depth += 1,
            b'>' if i > 0 && b[i - 1] == b'-' => {}
            b')' | b']' | b'>' => depth -= 1,
            b':' if depth == 0 && b.get(i + 1) != Some(&b':') && (i == 0 || b[i - 1] != b':') => {
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

/// The index after the `>` that closes the `<` at `open` (the `>` of `->` closes nothing).
fn close_angle(c: &[char], open: usize) -> usize {
    let mut depth = 0i32;
    let mut i = open;
    loop {
        match c[i] {
            '<' => depth += 1,
            '>' if c[i - 1] == '-' => {}
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// A `fn` head without parameter names: `pub fn get(&mut self, o: Local<'s, JsObject>)` ->
/// `pub fn get(&mut self, Local<'s, JsObject>)`.
fn fn_sig(head: &str) -> String {
    let c: Vec<char> = head.chars().collect();
    // `fn`, the name, the generics, then the parameter list.
    let Some(mut k) =
        (0..c.len()).find(|&i| c[i..].starts_with(&['f', 'n', ' ']) && (i == 0 || c[i - 1] == ' '))
    else {
        return head.to_string();
    };
    k += 3;
    while k < c.len() && (is_ident(c[k]) || c[k] == ' ') {
        k += 1;
    }
    if c.get(k) == Some(&'<') {
        k = close_angle(&c, k);
    }
    while c.get(k) == Some(&' ') {
        k += 1;
    }
    if c.get(k) != Some(&'(') {
        return head.to_string();
    }
    let open = k;
    let end = close(&c, open);
    let params: String = c[open + 1..end - 1].iter().collect();
    let mut kept = Vec::new();
    for p in split_top(&params) {
        let (attrs, p) = split_attrs(&p);
        let selfish = p == "self"
            || p.ends_with("&self")
            || p.ends_with("mut self")
            || p.starts_with("self:")
            || p.starts_with("mut self:")
            || (p.starts_with('&') && p.ends_with(" self"));
        let ty = if selfish {
            p
        } else {
            match type_colon(&p) {
                Some(k) => norm(&p[k + 1..]),
                None => p,
            }
        };
        kept.push(format!("{}{ty}", kept_attrs(&attrs)));
    }
    let before: String = c[..open].iter().collect();
    let after: String = c[end..].iter().collect();
    // A `where` clause ends with a comma before the body.
    let sig = norm(&format!("{before}({}){after}", kept.join(", ")));
    sig.trim_end_matches(',').to_string()
}

/// The name of an impl's self type: `impl<'s> abi::Cx<'s>` -> `Cx`; `impl X for &str` ->
/// `&str`. And the trait name for a trait impl.
fn impl_target(head: &str) -> (Option<String>, String) {
    // Drop `impl` and its generics.
    let rest = head.trim_start_matches("impl");
    let rest = if rest.starts_with('<') {
        let c: Vec<char> = rest.chars().collect();
        let mut depth = 0;
        let mut k = 0;
        for (i, &ch) in c.iter().enumerate() {
            match ch {
                '<' => depth += 1,
                '>' if i > 0 && c[i - 1] == '-' => {}
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        k = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        c[k..].iter().collect::<String>()
    } else {
        rest.to_string()
    };
    let rest = norm(&rest);
    let last_name = |s: &str| {
        let s = s.split('<').next().unwrap_or("").trim();
        s.rsplit("::").next().unwrap_or("").to_string()
    };
    match rest.split_once(" for ") {
        Some((tr, ty)) => (Some(last_name(tr)), last_name(ty)),
        None => (None, last_name(&rest)),
    }
}

/// `abi::Cx<'s>` -> `Cx<'s>`: module paths of the crate's own items are not part of a line.
fn drop_paths(s: &str) -> String {
    s.replace("crate::abi::", "")
        .replace("abi::", "")
        .replace("crate::", "")
}

fn is_pub(head: &str) -> bool {
    head.starts_with("pub ")
}

/// The name an item declares: `pub struct Args<'s>` -> `Args`.
fn item_name(head: &str) -> Option<String> {
    let words: Vec<&str> = head.split(' ').collect();
    for (k, w) in words.iter().enumerate() {
        if ["struct", "enum", "trait", "type", "fn", "const", "static"].contains(w) {
            let n = words.get(k + 1)?;
            let n: String = n.chars().take_while(|&ch| is_ident(ch)).collect();
            return Some(n);
        }
    }
    None
}

/// The `Isolate` methods in the core (H20-Q4, M3-P3 FREEZE-PREP step 2): the embedder's ways
/// into ABI code. A `Cx` scope (`with_cx`, `with_cx_checkpoint`, `create_realm`), the job run
/// (`perform_microtask_checkpoint`), and the hooks that ABI code calls (`register_host_function`
/// takes a `HostFn`; `with_module_host` takes the hook types of `Cx::run_module`). The other
/// `Isolate` methods (`new`, the handles, the heap, `string_view`, ...) are the embedder API of
/// H22, H23 and H19, with their own freeze points, so they are not here.
const ENTRY_POINTS: [&str; 6] = [
    "create_realm",
    "perform_microtask_checkpoint",
    "register_host_function",
    "with_cx",
    "with_cx_checkpoint",
    "with_module_host",
];

/// The append-only lists that have their own goldens (H20 part 2): their variants and their
/// `ALL` values are not core lines.
const PART2_ENUMS: [&str; 2] = ["IntrinsicId", "ObjectClass"];

/// The lines of one public item `it` (not an impl) of group `g`.
fn item_lines(g: &str, it: &Item, out: &mut Vec<String>) {
    let attrs = kept_attrs(&it.attrs);
    let head = drop_paths(&it.head);
    let name = item_name(&it.head).unwrap_or_default();
    if head.contains(" struct ") || head.starts_with("pub struct") {
        let mut pub_fields = Vec::new();
        let mut private = false;
        let mut head = head.clone();
        if let Some(b) = head.find('(') {
            // A tuple struct: its fields are in the head.
            let c: Vec<char> = head.chars().collect();
            let open = head[..b].chars().count();
            let end = close(&c, open);
            let inner: String = c[open + 1..end - 1].iter().collect();
            let mut shown = Vec::new();
            for f in split_top(&inner) {
                if f.starts_with("pub ") {
                    shown.push(f);
                } else {
                    shown.push("_".to_string());
                }
            }
            head = format!(
                "{}({})",
                c[..open].iter().collect::<String>(),
                shown.join(", ")
            );
        }
        if let Some(body) = &it.body {
            for f in split_top(body) {
                let (fa, f) = split_attrs(&f);
                if f.starts_with("pub ") {
                    pub_fields.push(format!("{}{f}", kept_attrs(&fa)));
                } else {
                    private = true;
                }
            }
            if private {
                pub_fields.push("..".to_string());
            }
            out.push(format!(
                "{g}: {attrs}{head} {{ {} }}",
                pub_fields.join(", ")
            ));
        } else {
            out.push(format!("{g}: {attrs}{head}"));
        }
    } else if head.starts_with("pub enum") {
        let body = it.body.clone().unwrap_or_default();
        let variants = if PART2_ENUMS.contains(&name.as_str()) {
            vec![format!(".. (part 2: the {name} golden)")]
        } else {
            split_top(&body)
                .into_iter()
                .map(|v| {
                    let (va, v) = split_attrs(&v);
                    format!("{}{v}", kept_attrs(&va))
                })
                .collect()
        };
        out.push(norm(&format!(
            "{g}: {attrs}{head} {{ {} }}",
            variants.join(", ")
        )));
    } else if head.starts_with("pub trait") {
        let mut fns = Vec::new();
        for m in items(it.body.as_deref().unwrap_or("")) {
            fns.push(format!(
                "{}{}",
                kept_attrs(&m.attrs),
                drop_paths(&fn_sig(&m.head))
            ));
        }
        out.push(format!("{g}: {attrs}{head} {{ {} }}", fns.join("; ")));
    } else if head.starts_with("pub type") {
        out.push(format!(
            "{g}: {attrs}{}",
            drop_paths(&fn_sig_in_type(&head))
        ));
    } else if is_fn_head(&head) {
        out.push(format!("{g}: {attrs}{}", drop_paths(&fn_sig(&head))));
    } else if head.starts_with("pub const") || head.starts_with("pub static") {
        out.push(format!(
            "{g}: {attrs}{}",
            const_line(&head, it.body.as_deref(), "")
        ));
    }
}

/// A type alias as it is (a fn pointer type names no parameters).
fn fn_sig_in_type(head: &str) -> String {
    head.to_string()
}

/// `pub const NAME: T = value` with its value; for the `ALL` list of a part-2 enum, the
/// count and the entries are not core.
fn const_line(head: &str, body: Option<&str>, owner: &str) -> String {
    let head = drop_paths(head);
    if PART2_ENUMS.contains(&owner) && item_name(&head).as_deref() == Some("ALL") {
        let ty = head.split_once(':').map(|(_, t)| t).unwrap_or("");
        let ty = ty.split(" = ").next().unwrap_or("").trim();
        let ty: String = match ty.rfind(';') {
            Some(k) if ty.starts_with('[') => format!("{}; N]", &ty[..k]),
            _ => ty.to_string(),
        };
        let name = head.split(':').next().unwrap_or("");
        return format!("{name}: {ty} = .. (part 2: the {owner} golden)");
    }
    match body {
        Some(b) => format!("{head} {{ {} }}", split_top(b).join(", ")),
        None => head,
    }
}

/// Every line of the golden from `files` ((path under `src/`, text)).
fn extract(files: &[(String, String)]) -> Vec<String> {
    let stripped: Vec<(String, String)> =
        files.iter().map(|(p, t)| (p.clone(), strip(t))).collect();
    let group = |p: &str| if p == "builtins.rs" { "tables" } else { "abi" };
    // The public types and traits of the ABI files: their impls are part of the ABI.
    let mut pub_types = BTreeSet::new();
    let mut pub_traits = BTreeSet::new();
    for (p, t) in &stripped {
        if !(p.starts_with("abi/") || p == "builtins.rs") {
            continue;
        }
        for it in items(t) {
            if !is_pub(&it.head) {
                continue;
            }
            let Some(n) = item_name(&it.head) else {
                continue;
            };
            if it.head.contains("trait ") {
                pub_traits.insert(n);
            } else if ["struct ", "enum ", "type "]
                .iter()
                .any(|k| it.head.contains(k))
            {
                pub_types.insert(n);
            }
        }
    }
    let mut out = Vec::new();
    let mut abi_mod_uses: BTreeSet<String> = BTreeSet::new();
    for (p, t) in &stripped {
        let g = group(p);
        let is_abi = p.starts_with("abi/") || p == "builtins.rs";
        for it in items(t) {
            if p == "lib.rs" {
                lib_paths(&it, &mut out);
                continue;
            }
            if p == "abi/mod.rs" && it.head.starts_with("pub use ") {
                for n in use_names(&use_text(&it)) {
                    let name = n.rsplit("::").next().unwrap().to_string();
                    let note = if name == "export_facts" {
                        " (part 2, not core: V8-A13)"
                    } else {
                        ""
                    };
                    out.push(format!("path: ep9_vm::abi::{name}{note}"));
                    abi_mod_uses.insert(name);
                }
                continue;
            }
            if p == "isolate.rs" && it.head == "impl Isolate" && kept_attrs(&it.attrs).is_empty() {
                // The embedder entry points (not a `cfg(test)` or spike-native block).
                for m in items(it.body.as_deref().unwrap_or("")) {
                    let name = item_name(&m.head).unwrap_or_default();
                    if is_pub(&m.head) && ENTRY_POINTS.contains(&name.as_str()) {
                        let sig = drop_paths(&fn_sig(&m.head));
                        out.push(format!(
                            "embed: impl Isolate :: {}{sig}",
                            kept_attrs(&m.attrs)
                        ));
                    }
                }
                continue;
            }
            if it.head.starts_with("impl") {
                let (tr, ty) = impl_target(&it.head);
                let ours =
                    pub_types.contains(&ty) || tr.as_ref().is_some_and(|t| pub_traits.contains(t));
                if !ours || (!is_abi && ty != "Cx") {
                    continue;
                }
                let ih = drop_paths(&it.head);
                if tr.is_some() {
                    out.push(format!("{g}: {ih}"));
                    continue;
                }
                for m in items(it.body.as_deref().unwrap_or("")) {
                    if !is_pub(&m.head) {
                        continue;
                    }
                    let a = kept_attrs(&m.attrs);
                    let line = if is_fn_head(&m.head) {
                        drop_paths(&fn_sig(&m.head))
                    } else {
                        const_line(&m.head, m.body.as_deref(), &ty)
                    };
                    out.push(format!("{g}: {ih} :: {a}{line}"));
                }
                continue;
            }
            if !is_abi {
                continue;
            }
            if it.head.ends_with('!') && !it.head.starts_with("macro_rules") {
                // A macro call at the top level: the marker lists of local.rs.
                let name = it.head.trim_end_matches('!');
                let entries: Vec<String> = split_top(it.body.as_deref().unwrap_or(""))
                    .into_iter()
                    .map(|e| {
                        if name == "downcast" {
                            e.split(" =>").next().unwrap_or("").to_string()
                        } else {
                            e
                        }
                    })
                    .collect();
                out.push(format!("{g}: {name}! {{ {} }}", entries.join(", ")));
                continue;
            }
            if is_pub(&it.head) && !it.head.starts_with("pub use") {
                item_lines(g, &it, &mut out);
                if p == "abi/mod.rs"
                    && let Some(n) = item_name(&it.head)
                {
                    out.push(format!("path: ep9_vm::abi::{n}"));
                    abi_mod_uses.insert(n);
                }
            }
        }
    }
    // A pub type of an `abi` submodule that `abi/mod.rs` does not re-export has no path.
    for (p, t) in &stripped {
        if !p.starts_with("abi/") || p == "abi/mod.rs" {
            continue;
        }
        for it in items(t) {
            if !is_pub(&it.head) || it.head.starts_with("pub use") {
                continue;
            }
            let Some(n) = item_name(&it.head) else {
                continue;
            };
            if !abi_mod_uses.contains(&n) {
                out.push(format!(
                    "path: (none) {n}: pub in the private module abi::{}, not re-exported",
                    p.trim_start_matches("abi/").trim_end_matches(".rs")
                ));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The names of a `pub use a::{B, C}` or `pub use a::B` head.
fn use_names(head: &str) -> Vec<String> {
    let rest = head.trim_start_matches("pub use ").trim();
    if let (Some(a), Some(b)) = (rest.find('{'), rest.rfind('}')) {
        let prefix = &rest[..a];
        split_top(&rest[a + 1..b])
            .into_iter()
            .flat_map(|n| {
                if n.contains('{') {
                    use_names(&format!("pub use {prefix}{n}"))
                } else {
                    vec![format!("{prefix}{n}")]
                }
            })
            .collect()
    } else {
        vec![rest.to_string()]
    }
}

/// The text of a `use` item: `items` stops a head at `{`, so `pub use x::{..};` comes as a
/// head and a body.
fn use_text(it: &Item) -> String {
    match &it.body {
        Some(b) => format!("{}{{{b}}}", it.head),
        None => it.head.clone(),
    }
}

/// The crate-root paths of `abi` and `builtins` names from a `lib.rs` item.
fn lib_paths(it: &Item, out: &mut Vec<String>) {
    if !it.head.starts_with("pub use ") {
        return;
    }
    for n in use_names(&use_text(it)) {
        let n = norm(&n);
        let module = n.split("::").next().unwrap_or("");
        if module == "abi" || module == "builtins" || n == "isolate::Isolate" {
            let name = n.rsplit("::").next().unwrap();
            out.push(format!("path: ep9_vm::{name} = {n}"));
        }
    }
}

// ---- the golden ------------------------------------------------------------------------------

const HEADER: &str = "\
# H20 core: the public names and signatures of the builtin ABI (docs/adr/H20-builtin-abi.md,
# Freeze table). Frozen at M0 exit: after it, a changed or removed line is a change to a
# Frozen part; a new line is an addition (part 2, append only). Made by
# `EP9_BLESS=1 cargo test -p ep9_vm --test h20_core`; see crates/ep9_vm/tests/h20_core.rs.
";

fn text(lines: &[String]) -> String {
    let mut s = HEADER.to_string();
    for l in lines {
        s.push_str(l);
        s.push('\n');
    }
    s
}

fn golden_lines() -> Vec<String> {
    std::fs::read_to_string(golden_path())
        .unwrap_or_else(|e| panic!("{}: {e}; run with EP9_BLESS=1", golden_path().display()))
        .replace("\r\n", "\n")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(String::from)
        .collect()
}

/// (lines of the golden that the source no longer has, lines of the source that are new).
fn diff(golden: &[String], now: &[String]) -> (Vec<String>, Vec<String>) {
    let g: BTreeSet<&String> = golden.iter().collect();
    let n: BTreeSet<&String> = now.iter().collect();
    (
        g.difference(&n).map(|s| s.to_string()).collect(),
        n.difference(&g).map(|s| s.to_string()).collect(),
    )
}

#[test]
fn h20_core_matches_the_golden() {
    let now = extract(&sources());
    if std::env::var_os("EP9_BLESS").is_some() {
        std::fs::write(golden_path(), text(&now)).unwrap();
        return;
    }
    let (gone, new) = diff(&golden_lines(), &now);
    assert!(
        gone.is_empty() && new.is_empty(),
        "the H20 core changed.\n\
         changed or removed (the core; after M0 exit a Frozen-part change, README §3.2):\n  {}\n\
         new (an addition, append only; review it, then run with EP9_BLESS=1):\n  {}",
        gone.join("\n  "),
        new.join("\n  ")
    );
}

/// The core has its parts: `HostFn`, `Args`, `Local`, the `Cx` methods (also the scoped
/// embedder forms of isolate.rs), the table types, the public paths.
#[test]
fn the_golden_covers_the_core() {
    let lines = golden_lines();
    let has = |s: &str| lines.iter().any(|l| l.contains(s));
    for needle in [
        "pub type HostFn = for<'s> fn(&mut Cx<'s>, Local<'s>, Args<'s>, Local<'s>) -> JsResult<Local<'s>>",
        "impl<'s> Args<'s> :: pub fn len(&self) -> usize",
        "pub struct Local<'s, T = JsValue> { .. }",
        "impl<'s> Cx<'s> :: pub fn run_script(&mut self, VerifiedUnit, &str) -> JsResult<Local<'s>>",
        "impl<'s> Cx<'s> :: pub fn get_index(",
        "tables: #[derive(Clone, Copy)] pub struct BuiltinDef { pub name: &'static str,",
        "path: ep9_vm::Cx = abi::Cx",
        "path: ep9_vm::abi::export_facts (part 2, not core: V8-A13)",
        "path: ep9_vm::CollectionCursor = abi::CollectionCursor",
        "path: ep9_vm::Isolate = isolate::Isolate",
    ] {
        assert!(has(needle), "no line has {needle:?}");
    }
    let cx = lines
        .iter()
        .filter(|l| l.contains("impl<'s> Cx<'s> :: pub fn"))
        .count();
    assert!(cx > 100, "{cx} Cx methods");
    assert!(
        !lines.iter().any(|l| l.starts_with("path: (none)")),
        "an ABI type with no public path"
    );
}

/// Each embedder entry point (H20-Q4) has one golden line, and no other `Isolate` method has
/// one: a rename or a removal of an entry point fails here and in the golden.
#[test]
fn the_golden_has_the_embedder_entry_points() {
    let lines = golden_lines();
    let embed: Vec<&String> = lines.iter().filter(|l| l.starts_with("embed: ")).collect();
    for name in ENTRY_POINTS {
        let n = embed
            .iter()
            .filter(|l| {
                l.contains(&format!(":: pub fn {name}<"))
                    || l.contains(&format!(":: pub fn {name}("))
            })
            .count();
        assert_eq!(n, 1, "{name}: {embed:?}");
    }
    assert_eq!(embed.len(), ENTRY_POINTS.len(), "{embed:?}");
}

/// The negative control: a changed return type is a changed core line (the old line is gone,
/// a new one is there).
#[test]
fn changing_a_signature_fails_the_golden() {
    let mut files = sources();
    let (_, t) = files.iter_mut().find(|(p, _)| p == "abi/mod.rs").unwrap();
    let from = "pub fn len(&self) -> usize";
    assert!(t.contains(from));
    *t = t.replacen(from, "pub fn len(&self) -> u32", 1);
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    assert_eq!(
        gone,
        ["abi: impl<'s> Args<'s> :: pub fn len(&self) -> usize"]
    );
    assert_eq!(new, ["abi: impl<'s> Args<'s> :: pub fn len(&self) -> u32"]);
}

/// The negative control for the embedder entry points: a changed `Isolate` entry point is a
/// changed core line, and a test-only `impl Isolate` block adds no line.
#[test]
fn changing_an_entry_point_fails_the_golden() {
    let mut files = sources();
    let (_, t) = files.iter_mut().find(|(p, _)| p == "isolate.rs").unwrap();
    let from = "pub fn register_host_function(&mut self, name: &str, length: u32, f: HostFn)";
    assert!(t.contains(from));
    *t = t.replacen(
        from,
        "pub fn register_host_function(&mut self, name: &str, length: u64, f: HostFn)",
        1,
    );
    t.push_str("\n#[cfg(test)]\nimpl Isolate {\n    pub fn with_cx(&self) {}\n}\n");
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    assert_eq!(
        gone,
        ["embed: impl Isolate :: pub fn register_host_function(&mut self, &str, u32, HostFn)"]
    );
    assert_eq!(
        new,
        ["embed: impl Isolate :: pub fn register_host_function(&mut self, &str, u64, HostFn)"]
    );
}

/// A new `Cx` method is an addition only: no golden line goes away.
#[test]
fn a_new_cx_method_is_an_addition() {
    let mut files = sources();
    let (_, t) = files.iter_mut().find(|(p, _)| p == "abi/time.rs").unwrap();
    let from = "impl<'s> Cx<'s> {";
    assert!(t.contains(from));
    *t = t.replacen(
        from,
        "impl<'s> Cx<'s> {\n    /// A probe.\n    pub fn probe_method(\n        &mut self,\n        v: Local<'s>,\n    ) -> bool {\n        let _ = v;\n        true\n    }\n",
        1,
    );
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    assert!(gone.is_empty(), "{gone:?}");
    assert_eq!(
        new,
        ["abi: impl<'s> Cx<'s> :: pub fn probe_method(&mut self, Local<'s>) -> bool"]
    );
}

/// A parameter rename, a moved method, a doc change and a body change keep the golden.
#[test]
fn renames_and_bodies_keep_the_golden() {
    let mut files = sources();
    let (_, t) = files.iter_mut().find(|(p, _)| p == "abi/mod.rs").unwrap();
    let from = "pub fn get(&self, i: usize) -> Local<'s> {";
    assert!(t.contains(from));
    *t = t.replacen(
        from,
        "/// Changed text { [ ( \" '{'\n    pub fn get(&self, index_renamed: usize) -> Local<'s> {\n        let _ = \"}}}\";",
        1,
    );
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    assert!(gone.is_empty() && new.is_empty(), "{gone:?} {new:?}");
}

/// The extractor on a small source: literals and comments with brackets, a lifetime, a
/// multi-line signature with a `where` clause, private and `pub(crate)` items, a private
/// module, a trait impl, a tuple struct and an enum.
#[test]
fn the_extractor_reads_rust_items() {
    let src = r##"
//! Docs { [ (
use x::y;
/* a { block /* nested } */ comment */
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct T<'s>(pub u64, u32);
pub struct S { pub a: u8, b: u16, #[cfg(test)] pub c: u8 }
#[non_exhaustive]
pub enum E { A, B(u8), C { x: u8 } = 3 }
pub struct Cx<'s> { inner: &'s u8 }
struct Hidden;
mod private { pub fn no() {} }
impl<'s> Cx<'s> {
    /// Text with '{' and "}" and r#"}"#.
    #[inline]
    pub fn one<F: Fn(u8) -> u8>(
        &mut self,
        mut a: Local<'s, JsObject>,
        (b, c): (u8, u8),
        f: F,
    ) -> JsResult<()>
    where
        F: Copy,
    {
        let s = "{"; let ch = '}'; let l: &'static str = r"}"; let e = '\'';
        Ok(())
    }
    pub(crate) fn two(&self) {}
    fn three(&self) {}
    pub const K: u32 = 1 << 3;
}
impl<T> Clone for T<'_> { fn clone(&self) -> Self { *self } }
impl Hidden { pub fn not_shown(&self) {} }
pub fn free(x: &str) -> bool { x.is_empty() }
"##;
    let stripped = strip(src);
    assert!(!stripped.contains("Docs") && !stripped.contains("nested"));
    let files = vec![("abi/probe.rs".to_string(), src.to_string())];
    let lines = extract(&files);
    let want = [
        "abi: #[derive(Clone, Copy)] #[repr(transparent)] pub struct T<'s>(pub u64, _)",
        "abi: #[non_exhaustive] pub enum E { A, B(u8), C { x: u8 } = 3 }",
        "abi: impl<'s> Cx<'s> :: pub const K: u32 = 1 << 3",
        "abi: impl<'s> Cx<'s> :: pub fn one<F: Fn(u8) -> u8>(&mut self, Local<'s, JsObject>, (u8, u8), F) -> JsResult<()> where F: Copy",
        "abi: impl<T> Clone for T<'_>",
        "abi: pub fn free(&str) -> bool",
        "abi: pub struct S { pub a: u8, #[cfg(test)] pub c: u8, .. }",
    ];
    let mut want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
    // The probe file is not `abi/mod.rs`, so its pub items have no path.
    for n in ["Cx", "E", "S", "T", "free"] {
        want.push(format!(
            "path: (none) {n}: pub in the private module abi::probe, not re-exported"
        ));
    }
    want.push("abi: pub struct Cx<'s> { .. }".to_string());
    want.sort();
    assert_eq!(lines, want);
}

/// The public path rule (H20 "Public paths"; M3-P3 FREEZE-PREP task 3): every name of
/// `ep9_vm::abi` is also a crate-root name, except `export_facts` (part 2, V8-A13).
#[test]
fn every_abi_name_has_a_crate_root_path() {
    let lines = extract(&sources());
    let abi: BTreeSet<String> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("path: ep9_vm::abi::"))
        .filter(|n| !n.starts_with("export_facts"))
        .map(String::from)
        .collect();
    let root: BTreeSet<String> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("path: ep9_vm::"))
        .filter_map(|l| l.split_once(" = abi::"))
        .map(|(n, _)| n.to_string())
        .collect();
    let missing: Vec<&String> = abi.difference(&root).collect();
    assert!(
        missing.is_empty(),
        "no crate-root re-export in lib.rs for {missing:?}"
    );
}
