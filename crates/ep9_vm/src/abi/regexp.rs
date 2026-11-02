//! RegExp on the builtin ABI (owners M4-P2 RX0A, then M4-P3 REGEXP1; V9-D15): the part-2 `Cx`
//! methods through which `ep9_builtins` reaches `ep9_regexp` (a dependency of `ep9_vm` only,
//! dep-guards check 14), and the match record they return.
//!
//! ```text
//! ep9_builtins regexp.rs ──Cx::regexp_create / Cx::regexp_exec (RX0A)──> regexp.rs (VM)
//!   (the RegExp builtins, REGEXP1)                                        └─> ep9_regexp
//!   MatchRecord: spans + group names (REGEXP1); Cx::is_builtin: the fast-path test (REGEXP1)
//! ```
//!
//! | Method | Spec | Errors |
//! |---|---|---|
//! | `regexp_create(proto, pattern, flags)` | RegExpAlloc (§22.2.3.1) with the prototype `proto`, then RegExpInitialize (§22.2.3.3) for the strings `pattern` and `flags` (the caller ran ToString, after its GetPrototypeFromConstructor, so the spec order holds): the slots, `lastIndex` = 0, the program compiled now | SyntaxError for a bad flags string (a letter not in `dgimsuvy`, a letter twice, `u` with `v`) or a pattern that does not compile |
//! | `regexp_exec(re, s, index)` | the matcher step of RegExpBuiltinExec (§22.2.7.2): the first match at or after code unit `index` (sticky: only at `index`; Unicode mode: an `index` inside a surrogate pair starts at the pair, as V8); no `lastIndex` read or write, no result object. The [`MatchRecord`] also names the groups (`[[GroupName]]`, M4-P3) | TypeError if `re` is not a RegExp object; SyntaxError if a literal's pattern does not compile (literals compile at their first exec); the termination of an interrupt |
//! | `regexp_initialize(re, pattern, flags)` (M5-P2 REGEXP2) | RegExpInitialize (§22.2.3.3) steps 5-15 on an existing RegExp, for Annex B `compile` (B.2.4.1); the program from the cache or compiled now; no `lastIndex` step | TypeError if `re` is not a RegExp object; SyntaxError for bad flags or a pattern that does not compile (then `re` is unchanged) |
//! | `regexp_legacy_features(re)`, `regexp_disable_legacy_features(re)` (M5-P2) | `[[Realm]]` and `[[LegacyFeaturesEnabled]]` of the legacy RegExp features proposal (Annex B): the `compile` receiver test; RegExpAlloc for a NewTarget that is not `%RegExp%` | none |
//! | `regexp_note_match(re, s, m)`, `regexp_legacy_static(which)`, `regexp_set_legacy_input(s)` (M5-P2) | UpdateLegacyRegExpStaticProperties / InvalidateLegacyRegExpStaticProperties after a RegExpBuiltinExec match; GetLegacyRegExpStaticProperty and SetLegacyRegExpStaticProperty steps after the receiver check (`RegExp.input`, `$_`, `lastMatch`, `$&`, `lastParen`, `$+`, `leftContext`, `` $` ``, `rightContext`, `$'`, `$1`-`$9`) | `None` for the spec's `empty` (the caller throws) |
//! | `is_builtin(f, id)` (M4-P3 REGEXP1) | whether `f` is the current realm's function object of builtin `id`: the "still the intrinsic?" test of a fast path (the RegExp builtins take theirs only while `%RegExp.prototype%.exec` is the builtin `exec`, the exec fuse of H31) | none (no allocation, no JS) |
//! | `regexp_has_initial_flags(rx, getters)` (M5-P3 RX0b) | whether `? ToString(? Get(rx, "flags"))` (§22.2.6.4) is the text of `rx`'s flags with nothing observable: `rx` has the initial RegExp shape and the current realm's `%RegExp.prototype%`, whose 9 flag accessors are the builtins `getters` (checked once, then kept by a kind-1 fuse of H31) | none (false when it does not hold; no JS) |
//! | `regexp_push_units(s, from, to, out)` (M5-P3 RX0b) | the code units `from..to` of `s` (a substring of S) appended to a Rust buffer, read in place: @@replace keeps no copy of S | none (no JS; a panic for a range past the end) |
//!
//! Each method is a part-2 addition: an `h20-core.txt` line (EP9_BLESS) and an H20 Change-log
//! row (V9-A1, V9-A14).

use ep9_value::JsResult;

use super::{Cx, JsObject, JsString, Local};
use crate::builtins::{self, BuiltinId};
use crate::frame::NULL;
use crate::kinds;
use crate::regexp::{self, Names};
use crate::string::{self, JsStr};

/// The result of one successful match: the capture spans in UTF-16 code units (index 0 is the
/// whole match; `None` for a capture that did not take part) and the group names of the
/// pattern. Rust memory with no Value in it (plan rule 8), so it may outlive a GC point.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MatchRecord {
    captures: Vec<Option<(usize, usize)>>,
    /// `[[GroupName]]` of group `i` at `i - 1`; empty if no group has a name.
    names: Names,
}

impl MatchRecord {
    /// The start of the whole match (a code unit index).
    pub fn start(&self) -> usize {
        self.get(0).map_or(0, |(s, _)| s)
    }

    /// The end of the whole match (one past its last code unit).
    pub fn end(&self) -> usize {
        self.get(0).map_or(0, |(_, e)| e)
    }

    /// The number of capture groups of the pattern (not counting the whole match).
    pub fn group_count(&self) -> usize {
        self.captures.len().saturating_sub(1)
    }

    /// The span of capture `i` (0 = the whole match), or `None` if it did not take part or
    /// `i` > [`MatchRecord::group_count`].
    pub fn get(&self, i: usize) -> Option<(usize, usize)> {
        self.captures.get(i).copied().flatten()
    }

    /// True if a group of the pattern has a name (`(?<name>...)`): the match result then has
    /// a `groups` object (§22.2.7.2 step 29).
    pub fn has_group_names(&self) -> bool {
        !self.names.is_empty()
    }

    /// The `[[GroupName]]` of capture group `i` (1 = the first group) as UTF-16 code units, or
    /// `None` for an unnamed group, `i` = 0 or `i` > [`MatchRecord::group_count`]. With
    /// duplicate named groups (`(?<a>x)|(?<a>y)`) several groups have the same name.
    pub fn group_name(&self, i: usize) -> Option<&[u16]> {
        let n = self.names.get(i.checked_sub(1)?)?;
        n.as_deref()
    }
}

impl<'s> Cx<'s> {
    /// RegExpAlloc + RegExpInitialize (§22.2.3.1, §22.2.3.3): a new RegExp object with
    /// prototype `proto` (an object or null; the RegExp constructor passes the result of
    /// [`Cx::get_prototype_from_constructor`]), pattern `pattern` and flags `flags`, its
    /// program compiled now. SyntaxError for bad flags or a pattern that does not compile.
    /// Panics if `proto` is not an object or null.
    pub fn regexp_create(
        &mut self,
        proto: impl Into<Local<'s>>,
        pattern: impl Into<Local<'s, JsString>>,
        flags: impl Into<Local<'s, JsString>>,
    ) -> JsResult<Local<'s, JsObject>> {
        let p = proto.into().bits();
        assert!(
            p == NULL || kinds::is_object(p),
            "a prototype is an object or null"
        );
        let (pattern, flags) = (pattern.into().bits(), flags.into().bits());
        let o = regexp::create(self.inner(), p, pattern, flags)?;
        Ok(Self::local(o))
    }

    /// The first match of the RegExp object `re` in `s` at or after code unit `index` (a
    /// sticky RegExp: only at `index`), or `None` (also when `index` > the length): the
    /// matcher step of RegExpBuiltinExec (§22.2.7.2). It reads and writes no property
    /// (`lastIndex` is the caller's). TypeError if `re` is not a RegExp object.
    pub fn regexp_exec(
        &mut self,
        re: impl Into<Local<'s>>,
        s: impl Into<Local<'s, JsString>>,
        index: usize,
    ) -> JsResult<Option<MatchRecord>> {
        let (re, s) = (re.into().bits(), s.into().bits());
        let m = regexp::exec(self.inner(), re, s, index)?;
        Ok(m.map(|(captures, names)| MatchRecord { captures, names }))
    }

    /// The slot steps of RegExpInitialize (§22.2.3.3) on the existing RegExp object `re`, for
    /// Annex B `RegExp.prototype.compile` (B.2.4.1): the strings `pattern` and `flags` (the
    /// caller ran ToString) give `[[OriginalSource]]`, `[[OriginalFlags]]`,
    /// `[[RegExpRecord]]` and a new `[[RegExpMatcher]]`. The flags and the pattern are
    /// checked first: a SyntaxError leaves `re` as it was. No `lastIndex` step (the caller
    /// runs `Set(re, "lastIndex", 0, true)`). TypeError if `re` is not a RegExp object.
    pub fn regexp_initialize(
        &mut self,
        re: impl Into<Local<'s>>,
        pattern: impl Into<Local<'s, JsString>>,
        flags: impl Into<Local<'s, JsString>>,
    ) -> JsResult<()> {
        let re = re.into().bits();
        let (pattern, flags) = (pattern.into().bits(), flags.into().bits());
        regexp::initialize(self.inner(), re, pattern, flags)
    }

    /// Whether `re` is a RegExp object made in the current realm (`[[Realm]]`) with
    /// `[[LegacyFeaturesEnabled]]` true (Annex B legacy RegExp features): the receiver checks
    /// of `RegExp.prototype.compile`. False for any other value.
    pub fn regexp_legacy_features(&self, re: impl Into<Local<'s>>) -> bool {
        regexp::legacy_features(self.rt(), re.into().bits())
    }

    /// Sets `[[LegacyFeaturesEnabled]]` of the RegExp object `re` to false: RegExpAlloc
    /// (legacy proposal) when NewTarget is not the current realm's `%RegExp%` (a subclass).
    /// [`Cx::regexp_create`] makes it true. No effect on another value.
    pub fn regexp_disable_legacy_features(&mut self, re: impl Into<Local<'s>>) {
        regexp::disable_legacy_features(self.rt(), re.into().bits());
    }

    /// The legacy step of RegExpBuiltinExec (legacy proposal) after the match `m` of `re` in
    /// `s` (and after the `lastIndex` Set): in `re`'s realm, UpdateLegacyRegExpStaticProperties
    /// of the current `%RegExp%` when `re` has `[[LegacyFeaturesEnabled]]`, else
    /// InvalidateLegacyRegExpStaticProperties; in another realm nothing. No allocation, no JS.
    pub fn regexp_note_match(
        &mut self,
        re: impl Into<Local<'s>>,
        s: impl Into<Local<'s, JsString>>,
        m: &MatchRecord,
    ) {
        let (re, s) = (re.into().bits(), s.into().bits());
        regexp::note_match(self.rt(), re, s, &m.captures);
    }

    /// The legacy static value `which` of the current realm's `%RegExp%`
    /// (GetLegacyRegExpStaticProperty steps 3-5, after the caller's receiver check): 0
    /// `[[RegExpInput]]`, 1 `[[RegExpLastMatch]]`, 2 `[[RegExpLastParen]]`, 3
    /// `[[RegExpLeftContext]]`, 4 `[[RegExpRightContext]]`, 5 ... 13 `[[RegExpParen1]]` ...
    /// `[[RegExpParen9]]`. `None` when the value is the spec's `empty` (the caller throws a
    /// TypeError); the empty String before the first match. Panics if `which` > 13.
    pub fn regexp_legacy_static(&mut self, which: u32) -> JsResult<Option<Local<'s, JsString>>> {
        Ok(regexp::legacy_value(self.rt(), which).map(Self::local))
    }

    /// SetLegacyRegExpStaticProperty(%RegExp%, thisValue, `[[RegExpInput]]`, val) step 3,
    /// after the caller's receiver check and ToString: `[[RegExpInput]]` of the current
    /// realm's `%RegExp%` becomes `s`.
    pub fn regexp_set_legacy_input(&mut self, s: impl Into<Local<'s, JsString>>) {
        regexp::set_legacy_input(self.rt(), s.into().bits());
    }

    /// Whether `f` is the function object of the builtin `id` of the current realm (no
    /// allocation, no JS). False for a function of another realm, and in a realm after the
    /// first (the builtin table keeps the first realm's function objects only), so a fast path
    /// that tests this takes its spec steps there. The RegExp builtins test the exec fuse
    /// with it: `%RegExp.prototype%.exec` is still the builtin `exec` (MC-w14-18).
    pub fn is_builtin(&self, f: impl Into<Local<'s>>, id: BuiltinId) -> bool {
        let f = f.into().bits();
        kinds::is_function(f) && builtins::existing_function(self.rt(), id) == Some(f)
    }

    /// Whether `? ToString(? Get(rx, "flags"))` gives the flags of `rx`'s `[[OriginalFlags]]`
    /// (as bits: `regexp_exec`'s record) with no JS run and nothing observable, so a RegExp
    /// builtin may skip those Gets (M5-P3 RX0b): `rx` is a RegExp object with its initial
    /// shape (its only own property is `lastIndex`) whose `[[Prototype]]` is the current
    /// realm's `%RegExp.prototype%`, and there `flags`, `hasIndices`, `global`, `ignoreCase`,
    /// `multiline`, `dotAll`, `unicode`, `unicodeSets` and `sticky` are accessors whose
    /// getters are the builtins `getters` (in that order). The prototype check runs once and
    /// a kind-1 fuse (H31) keeps it until a structural change of the prototype chain; after
    /// that pop it is false for the life of the isolate. One `getters` list per isolate (a
    /// call with another list is false). False for any other value. GC point (the first
    /// call arms the fuse); no JS.
    pub fn regexp_has_initial_flags(
        &mut self,
        rx: impl Into<Local<'s>>,
        getters: &[BuiltinId; 9],
    ) -> bool {
        let rx = rx.into().bits();
        regexp::has_initial_flags(self.rt(), rx, getters)
    }

    /// Appends the code units `from..to` of the string `s` to `out`, read in place (a rope
    /// is flattened first): the subject reads of the RegExp builtins, which then keep no copy
    /// of the whole subject (M5-P3 RX0b). No JS. GC point (the flatten of a rope). Panics if
    /// `from > to` or `to` > the length of `s`.
    pub fn regexp_push_units(
        &mut self,
        s: impl Into<Local<'s, JsString>>,
        from: usize,
        to: usize,
        out: &mut Vec<u16>,
    ) {
        let flat = string::flatten(self.inner(), s.into().bits());
        match string::view(self.inner(), flat) {
            JsStr::Latin1(b) => out.extend(b[from..to].iter().map(|&u| u16::from(u))),
            JsStr::Utf16(w) => out.extend_from_slice(&w[from..to]),
        }
    }
}
