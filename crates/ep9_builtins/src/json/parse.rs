//! JSON.parse ( text [ , reviver ] ) (§25.5.1) and InternalizeJSONProperty (§25.5.1.1), with
//! json-parse-with-source: the JSON Parse Records (CreateJSONParseRecord) and the reviver's
//! `context` argument.
//!
//! ```text
//! parse    ToString(text) -> code units (a Rust Vec<u16>: no Values)
//!          one loop, no Rust recursion:
//!            value start  { [ -> a new container, pushed on `open`; " -> a string;
//!                         number, true, false, null -> a primitive
//!            value end    the top container takes it (CreateDataProperty), then , ] }
//!          open: a ScratchList (precise roots), 2 entries per open container:
//!                the container, and the pending key of an object (undefined for an array)
//!          the next index of each open array: a Rust Vec<u32>
//!          reviver callable -> the loop also makes the parse records: every value, every
//!            container and every key is appended to the ScratchList (nothing is reused), and
//!            a Rust Vec<Node> holds, per value, its list index and its source range or its
//!            elements / entries (list indices and node numbers only: no Values)
//! reviver  InternalizeJSONProperty over the result, depth first (recursive, as the spec;
//!          Cx::check_native_stack per level, Cx::poll_interrupt per property); a value that
//!          is SameValue to its record's value keeps the record (a primitive gets
//!          context.source, a container its element / entry records)
//! ```
//!
//! The grammar is ECMA-404 (§25.5.1 step 2 parses StringToCodePoints(jsonString) as a JSON
//! text): whitespace is tab, LF, CR and space only; no comments, no trailing commas, no
//! single quotes, no leading zeros, no `+`, `.5` or `5.`; a string holds no code unit below
//! U+0020 and lone surrogates pass through. A key `__proto__` is an ordinary own property
//! (CreateDataProperty), and a later duplicate key replaces the earlier value (its record is
//! the last one, as CreateJSONParseRecord picks the last property node of that name).

use ep9_vm::{Args, Cx, ErrorKind, FlatStr, JsKey, JsObject, JsResult, Local, ScratchList, Throw};

use crate::array::{index_key, length_of_array_like};

use super::{MAX_DEPTH, enumerable_own_keys, too_deep};

/// JSON.parse ( text [ , reviver ] ) (§25.5.1).
pub(super) fn parse<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    let reviver = args.get(1);
    // 1. Let jsonString be ? ToString(text).
    let json_string = cx.to_string(args.get(0))?;
    let flat = cx.flatten(json_string)?;
    let text = code_units(cx, flat);
    // 2-4. Parse StringToCodePoints(jsonString) as a JSON text (a SyntaxError if it is not
    //      one), and evaluate it: unfiltered. With a callable reviver the same loop makes the
    //      JSON Parse Records of 5.d (it runs no JS, so they are of the value as parsed).
    let list = cx.scratch_list();
    let track = reviver.is_callable();
    let mut parser = Parser::new(&text, track);
    let (unfiltered, root_node) = parser.run(cx, &list)?;
    // 5. If IsCallable(reviver) is true, then
    if track {
        // a. Let root be OrdinaryObjectCreate(%Object.prototype%).
        let root = cx.new_object();
        // b. Let rootName be the empty String.
        let root_name = cx.key("");
        // c. Perform ! CreateDataPropertyOrThrow(root, rootName, unfiltered).
        cx.create_data_property_or_throw(root, root_name, unfiltered)?;
        // d. Let snapshot be CreateJSONParseRecord(script, rootName, unfiltered).
        let records = Records {
            list: &list,
            nodes: &parser.nodes,
            text: flat,
            source: cx.key("source"),
        };
        // e. Return ? InternalizeJSONProperty(root, rootName, reviver, snapshot).
        return internalize(cx, &records, root, root_name, reviver, Some(root_node), 0);
    }
    // 6. Else, return unfiltered.
    Ok(unfiltered)
}

/// The code units of a flat string (a Rust Vec: no Values).
pub(super) fn code_units<'s>(cx: &Cx<'s>, flat: Local<'s, FlatStr>) -> Vec<u16> {
    let len = cx.string_len(flat);
    (0..len)
        .map(|i| cx.code_unit_at(flat, i).expect("i < length"))
        .collect()
}

/// A JSON Parse Record (json-parse-with-source): the parsed value (`[[Value]]`, list entry
/// `slot`), and its source range (`[[ParseNode]]` of a primitive) or its element or entry
/// records. Indices only, so Rust memory holds no Value (plan rule 8).
enum Node {
    /// A string, number, `true`, `false` or `null`: the text `start..end` (no whitespace).
    Primitive {
        slot: usize,
        start: usize,
        end: usize,
    },
    /// An array: `[[Elements]]`, one node per element in index order.
    Array { slot: usize, elements: Vec<u32> },
    /// An object: `[[Entries]]` as (list entry of the key atom, node), one per member in
    /// source order. `dup`: some key came more than once (the record of a key is then the
    /// last one with it).
    Object {
        slot: usize,
        entries: Vec<(usize, u32)>,
        dup: bool,
    },
}

impl Node {
    fn slot(&self) -> usize {
        match *self {
            Node::Primitive { slot, .. } | Node::Array { slot, .. } | Node::Object { slot, .. } => {
                slot
            }
        }
    }
}

/// The JSON Parse Records of one parse and what InternalizeJSONProperty reads with them.
struct Records<'a, 's> {
    /// The values and keys the nodes name (precise roots).
    list: &'a ScratchList<'s>,
    nodes: &'a [Node],
    /// jsonString, flat: the source text of a primitive is a substring of it.
    text: Local<'s, FlatStr>,
    /// The key `source`.
    source: Local<'s, JsKey>,
}

/// InternalizeJSONProperty ( holder, name, reviver, parseRecord ) (§25.5.1.1, with
/// json-parse-with-source). Recursive (one level per nesting level of the value; `depth`
/// levels are open), so each level checks the native stack and [`MAX_DEPTH`] first.
/// `record` is the node of parseRecord, or `None` for empty.
fn internalize<'s>(
    cx: &mut Cx<'s>,
    records: &Records<'_, 's>,
    holder: Local<'s, JsObject>,
    name: Local<'s, JsKey>,
    reviver: Local<'s>,
    record: Option<u32>,
    depth: usize,
) -> JsResult<Local<'s>> {
    cx.check_native_stack()?;
    if depth >= MAX_DEPTH {
        return Err(too_deep(cx));
    }
    // 1. Let val be ? Get(holder, name).
    let val = cx.get(holder, name)?;
    // 2. Let context be OrdinaryObjectCreate(%Object.prototype%).
    let context = cx.new_object();
    // 3. If parseRecord is a JSON Parse Record and SameValue(parseRecord.[[Value]], val) is
    //    true, then
    let node = record
        .map(|n| &records.nodes[n as usize])
        .filter(|node| cx.same_value(records.list.get(node.slot()), val));
    //    a. If val is not an Object, then
    //       i-iii. Let sourceText be the source text matched by parseRecord.[[ParseNode]] (not
    //              an array or object literal: a primitive's record has no other kind).
    //       iv. Perform ! CreateDataPropertyOrThrow(context, "source",
    //           CodePointsToString(sourceText)).
    if let Some(&Node::Primitive { start, end, .. }) = node {
        let source = cx.substring(records.text, start, end)?;
        cx.create_data_property_or_throw(context, records.source, source)?;
    }
    //    b. Let elementRecords be parseRecord.[[Elements]].
    //    c. Let entryRecords be parseRecord.[[Entries]].
    // 4. Else, let elementRecords and entryRecords be new empty Lists.
    let (elements, entries, dup): (&[u32], &[(usize, u32)], bool) = match node {
        Some(Node::Array { elements, .. }) => (elements, &[], false),
        Some(Node::Object { entries, dup, .. }) => (&[], entries, *dup),
        _ => (&[], &[], false),
    };
    // 5. If val is an Object, then
    if let Some(o) = val.as_object() {
        // a. Let isArray be ? IsArray(val).
        if cx.is_array_or_throw(o)? {
            // b. If isArray is true, then
            //    i. Let elementRecordsLen be the number of elements in elementRecords.
            //    ii. Let len be ? LengthOfArrayLike(val).
            let len = length_of_array_like(cx, o)?;
            //    iii. Let I be 0. iv. Repeat, while I < len,
            for i in 0..len {
                cx.poll_interrupt()?;
                // 1. Let prop be ! ToString(𝔽(I)).
                let prop = index_key(cx, i);
                // 2. If I < elementRecordsLen, let elementRecord be elementRecords[I];
                //    otherwise, let elementRecord be empty.
                let element = usize::try_from(i)
                    .ok()
                    .and_then(|i| elements.get(i).copied());
                // 3-5. Revive the element.
                revive_property(cx, records, o, prop, reviver, element, depth)?;
            }
        } else {
            // c. Else,
            //    i. Let keys be ? EnumerableOwnProperties(val, key).
            let (keys, n) = enumerable_own_keys(cx, o)?;
            let mut finder = EntryFinder::new(entries, dup);
            //    ii. For each String P of keys, do
            for i in 0..n {
                cx.poll_interrupt()?;
                let p = cx.get_index(keys, i)?;
                let p = cx.to_property_key(p)?;
                // 1. Let entryRecord be the element of entryRecords whose [[Key]] field is
                //    P. If there is no such element, let entryRecord be empty.
                let entry = finder.find(cx, records.list, p);
                // 2-4. Revive the property.
                revive_property(cx, records, o, p, reviver, entry, depth)?;
            }
        }
    }
    // 6. Return ? Call(reviver, holder, « name, val, context »).
    cx.call(reviver, holder, &[name.into(), val, context.into()])
}

/// InternalizeJSONProperty step 5.b.iv.3-5 and 5.c.ii.2-4 for the property `prop` of `val`.
fn revive_property<'s>(
    cx: &mut Cx<'s>,
    records: &Records<'_, 's>,
    val: Local<'s, JsObject>,
    prop: Local<'s, JsKey>,
    reviver: Local<'s>,
    record: Option<u32>,
    depth: usize,
) -> JsResult<()> {
    // 1. Let newElement be ? InternalizeJSONProperty(val, prop, reviver, record).
    let new_element = internalize(cx, records, val, prop, reviver, record, depth + 1)?;
    if new_element.is_undefined() {
        // 2. If newElement is undefined, then perform ? val.[[Delete]](prop).
        cx.delete(val, prop)?;
    } else {
        // 3. Else, perform ? CreateDataProperty(val, prop, newElement).
        cx.create_data_property(val, prop, new_element)?;
    }
    Ok(())
}

/// Finds the entry record of a key in one object's `[[Entries]]` (InternalizeJSONProperty
/// step 5.c.ii.1). The keys of an unchanged object come in the order of its entries (both
/// are the order the properties were made in), except that integer keys come first, so the
/// next entry is tried first. Past a miss, a small list is scanned and a long one gets a
/// table of key hashes, sorted (made once per object): no walk is quadratic in the number
/// of keys. Every match is by atom identity; with a duplicate key the last entry wins.
struct EntryFinder<'a> {
    entries: &'a [(usize, u32)],
    dup: bool,
    /// The entry after the last match.
    next: usize,
    /// (key hash, entry index), sorted, once a long list misses.
    table: Option<Vec<(u64, u32)>>,
}

/// A list this long or shorter is scanned on a miss.
const SCAN_ENTRIES: usize = 16;

impl<'a> EntryFinder<'a> {
    fn new(entries: &'a [(usize, u32)], dup: bool) -> EntryFinder<'a> {
        EntryFinder {
            entries,
            dup,
            next: 0,
            table: None,
        }
    }

    fn find<'s>(
        &mut self,
        cx: &mut Cx<'s>,
        list: &ScratchList<'s>,
        p: Local<'s, JsKey>,
    ) -> Option<u32> {
        let entries = self.entries;
        let matches = |j: usize| list.get(entries[j].0).is_same_bits(&p);
        // The next entry (no duplicate key, so it is the only one with its key).
        if !self.dup && self.next < entries.len() && matches(self.next) {
            self.next += 1;
            return Some(entries[self.next - 1].1);
        }
        let found = if entries.len() <= SCAN_ENTRIES {
            (0..entries.len()).rev().find(|&j| matches(j))
        } else {
            let table = self.table.get_or_insert_with(|| {
                let mut t: Vec<(u64, u32)> = (0..entries.len())
                    .map(|j| (key_hash(cx, list.get(entries[j].0)), j as u32))
                    .collect();
                t.sort_unstable();
                t
            });
            let h = key_hash(cx, p.value());
            let from = table.partition_point(|&(x, _)| x < h);
            table[from..]
                .iter()
                .take_while(|&&(x, _)| x == h)
                .map(|&(_, j)| j as usize)
                .filter(|&j| matches(j))
                .max()
        };
        let j = found?;
        self.next = j + 1;
        Some(entries[j].1)
    }
}

/// A hash of a key's code units (a symbol: 0; JSON keys are strings). Reads the units only:
/// no GC point (`Cx::flatten` of an atom, which is flat, allocates nothing).
fn key_hash<'s>(cx: &mut Cx<'s>, k: Local<'s>) -> u64 {
    let Some(s) = k.as_string() else {
        return 0;
    };
    let Ok(flat) = cx.flatten(s) else {
        return 0;
    };
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for i in 0..cx.string_len(flat) {
        let u = cx.code_unit_at(flat, i).expect("i < length");
        h = (h ^ u64::from(u)).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// CreateDataPropertyOrThrow(A, ! ToString(𝔽(i)), v) for the next index `i` of the new
/// array `a` (extensible, ordinary, no element at `i` or above). When no object on its
/// prototype chain has the index (HasProperty, which runs no JS and makes no atom), Set
/// finds nothing on the chain and adds the same own data property that CreateDataProperty
/// adds, so `Cx::set_index` stores the element without a key: a parse of a long array makes
/// no atom per index. Else the key path.
fn append_element<'s>(
    cx: &mut Cx<'s>,
    a: Local<'s, JsObject>,
    i: u64,
    v: Local<'s>,
) -> JsResult<()> {
    if !cx.has_index(a, i)? {
        cx.set_index(a, i, v, true)?;
        return Ok(());
    }
    let k = index_key(cx, i);
    cx.create_data_property_or_throw(a, k, v)
}

/// The JSON text, the read position, and the JSON Parse Records when they are made.
pub(super) struct Parser<'t> {
    text: &'t [u16],
    at: usize,
    /// Make the records: `nodes`, and a list entry of its own for every value and key.
    track: bool,
    /// The records, in the order the values start (tracking only).
    nodes: Vec<Node>,
}

/// One open container.
#[derive(Clone, Copy)]
struct Frame {
    /// The list entry of the container.
    container: usize,
    /// The list entry of the pending key of an object; [`NO_KEY`] for an array.
    key: usize,
    /// The next index of an array.
    next: u32,
    /// The container's record (tracking only).
    node: u32,
}

/// [`Frame::key`] of an array.
const NO_KEY: usize = usize::MAX;

/// The open containers, innermost last. Without tracking, depth `d` reuses the
/// [`ScratchList`] entries `2d` (the container) and `2d + 1` (the pending key of an object,
/// undefined for an array); entries past the depth are stale (rooted until the parse ends).
/// With tracking, every container and key gets a new entry, so the records can name it after
/// the parse.
struct Open<'s, 'l> {
    /// The innermost ScratchList while the parse runs (no JS runs), so `len` is its own.
    list: &'l ScratchList<'s>,
    frames: Vec<Frame>,
    track: bool,
}

impl<'s> Open<'s, '_> {
    /// Stores `v`: in entry `i` (reused) without tracking, else in a new entry. Gives the
    /// entry.
    fn store(&self, i: usize, v: Local<'s>) -> usize {
        if self.track {
            return self.list.push(v);
        }
        if i < self.list.len() {
            self.list.set(i, v);
        } else {
            debug_assert_eq!(i, self.list.len());
            self.list.push(v);
        }
        i
    }

    /// Opens `container` with its first key (an object) or none (an array). Gives the list
    /// entry of the container.
    fn push(&mut self, container: Local<'s>, key: Option<Local<'s>>) -> usize {
        let d = self.frames.len();
        let c = self.store(2 * d, container);
        let key = match key {
            Some(k) => self.store(2 * d + 1, k),
            None => {
                if !self.track {
                    self.store(2 * d + 1, Local::undefined());
                }
                NO_KEY
            }
        };
        self.frames.push(Frame {
            container: c,
            key,
            next: 0,
            node: 0,
        });
        c
    }

    /// The next key of the innermost object.
    fn set_key(&mut self, k: Local<'s>) {
        let d = self.frames.len() - 1;
        let i = self.store(2 * d + 1, k);
        self.frames[d].key = i;
    }
}

/// JSON whitespace (ECMA-404 §4): tab, LF, CR, space.
pub(super) fn is_ws(u: u16) -> bool {
    matches!(u, 0x09 | 0x0A | 0x0D | 0x20)
}

/// What started at a value start and is already whole.
enum Whole {
    Primitive,
    EmptyArray,
    EmptyObject,
}

impl<'t> Parser<'t> {
    /// A parser at the start of `text`; `track`: make the JSON Parse Records.
    pub(super) fn new(text: &'t [u16], track: bool) -> Parser<'t> {
        Parser {
            text,
            at: 0,
            track,
            nodes: Vec::new(),
        }
    }

    fn peek(&self) -> Option<u16> {
        self.text.get(self.at).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(is_ws) {
            self.at += 1;
        }
    }

    /// Appends a record; gives its number.
    fn add(&mut self, node: Node) -> u32 {
        self.nodes.push(node);
        u32::try_from(self.nodes.len() - 1).expect("fewer records than code units")
    }

    /// The SyntaxError at the current position (V8's texts).
    fn error<'s>(&self, cx: &mut Cx<'s>) -> Throw {
        let msg = match self.peek() {
            None => "Unexpected end of JSON input".to_string(),
            Some(u) => {
                let c = char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}');
                format!("Unexpected token '{c}' in JSON at position {}", self.at)
            }
        };
        cx.throw_error(ErrorKind::SyntaxError, &msg)
    }

    /// Consumes `u` (after whitespace), or a SyntaxError.
    fn expect<'s>(&mut self, cx: &mut Cx<'s>, u: u16) -> JsResult<()> {
        self.skip_ws();
        if self.peek() == Some(u) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.error(cx))
        }
    }

    /// Parses the whole text: one JSON value with optional whitespace around it. `list` is a
    /// new empty ScratchList, the innermost while the parse runs; with tracking it holds the
    /// records' values and keys afterwards. Gives the value and its record (0 without
    /// tracking).
    pub(super) fn run<'s>(
        &mut self,
        cx: &mut Cx<'s>,
        list: &ScratchList<'s>,
    ) -> JsResult<(Local<'s>, u32)> {
        let mut open = Open {
            list,
            frames: Vec::new(),
            track: self.track,
        };
        'value: loop {
            cx.poll_interrupt()?;
            // A value starts here.
            self.skip_ws();
            let start = self.at;
            let (mut value, whole): (Local<'s>, Whole) = match self.peek() {
                Some(0x7B) => {
                    // `{`: an object; `}` or the first key follows.
                    self.at += 1;
                    let o = cx.new_object();
                    self.skip_ws();
                    if self.peek() == Some(0x7D) {
                        self.at += 1;
                        (o.value(), Whole::EmptyObject)
                    } else {
                        let key = self.key(cx)?;
                        let slot = open.push(o.value(), Some(key.into()));
                        if self.track {
                            let node = self.add(Node::Object {
                                slot,
                                entries: Vec::new(),
                                dup: false,
                            });
                            open.frames.last_mut().expect("just pushed").node = node;
                        }
                        continue 'value;
                    }
                }
                Some(0x5B) => {
                    // `[`: an array; `]` or the first element follows.
                    self.at += 1;
                    let a = cx.new_array();
                    self.skip_ws();
                    if self.peek() == Some(0x5D) {
                        self.at += 1;
                        (a.value(), Whole::EmptyArray)
                    } else {
                        let slot = open.push(a.value(), None);
                        if self.track {
                            let node = self.add(Node::Array {
                                slot,
                                elements: Vec::new(),
                            });
                            open.frames.last_mut().expect("just pushed").node = node;
                        }
                        continue 'value;
                    }
                }
                Some(0x22) => (self.string(cx)?.value(), Whole::Primitive),
                Some(0x2D | 0x30..=0x39) => (Local::number(self.number(cx)?), Whole::Primitive),
                Some(0x74) => (
                    self.literal(cx, "true", Local::boolean(true))?,
                    Whole::Primitive,
                ),
                Some(0x66) => (
                    self.literal(cx, "false", Local::boolean(false))?,
                    Whole::Primitive,
                ),
                Some(0x6E) => (self.literal(cx, "null", Local::null())?, Whole::Primitive),
                _ => return Err(self.error(cx)),
            };
            // Its record (CreateJSONParseRecord: the value, and the source text of a
            // primitive).
            let mut node = 0;
            if self.track {
                let slot = list.push(value);
                node = self.add(match whole {
                    Whole::Primitive => Node::Primitive {
                        slot,
                        start,
                        end: self.at,
                    },
                    Whole::EmptyArray => Node::Array {
                        slot,
                        elements: Vec::new(),
                    },
                    Whole::EmptyObject => Node::Object {
                        slot,
                        entries: Vec::new(),
                        dup: false,
                    },
                });
            }
            // The value ends here: the open containers take it, innermost first, while they
            // close.
            loop {
                let Some(&frame) = open.frames.last() else {
                    self.skip_ws();
                    if self.at != self.text.len() {
                        return Err(self.error(cx));
                    }
                    return Ok((value, node));
                };
                let container = list
                    .get(frame.container)
                    .as_object()
                    .expect("open containers are objects");
                if frame.key == NO_KEY {
                    // An array element: CreateDataPropertyOrThrow(A, ! ToString(𝔽(i)), v)
                    // (ArrayLiteral evaluation), so no Array.prototype setter runs.
                    append_element(cx, container, u64::from(frame.next), value)?;
                    open.frames.last_mut().expect("open").next += 1;
                    // (No records without tracking: `nodes` is empty.)
                    if let Some(Node::Array { elements, .. }) =
                        self.nodes.get_mut(frame.node as usize)
                    {
                        elements.push(node);
                    }
                    self.skip_ws();
                    match self.peek() {
                        Some(0x2C) => {
                            self.at += 1;
                            continue 'value;
                        }
                        Some(0x5D) => self.at += 1,
                        _ => return Err(self.error(cx)),
                    }
                } else {
                    // An object member: CreateDataProperty (PropertyDefinitionEvaluation of
                    // a JSON object; `__proto__` is an ordinary key here).
                    let k = cx.to_property_key(list.get(frame.key))?;
                    if self.track {
                        // A key seen before in this object (HasOwnProperty of a new ordinary
                        // object: no JS runs).
                        let seen = cx.has_own_property(container, k)?;
                        if let Some(Node::Object { entries, dup, .. }) =
                            self.nodes.get_mut(frame.node as usize)
                        {
                            entries.push((frame.key, node));
                            *dup |= seen;
                        }
                    }
                    cx.create_data_property_or_throw(container, k, value)?;
                    self.skip_ws();
                    match self.peek() {
                        Some(0x2C) => {
                            self.at += 1;
                            self.skip_ws();
                            let key = self.key(cx)?;
                            open.set_key(key.into());
                            continue 'value;
                        }
                        Some(0x7D) => self.at += 1,
                        _ => return Err(self.error(cx)),
                    }
                }
                // The container closed: it is the value for the next one out.
                value = container.value();
                node = frame.node;
                open.frames.pop();
            }
        }
    }

    /// A member name and its `:`: the key (an atom string).
    fn key<'s>(&mut self, cx: &mut Cx<'s>) -> JsResult<Local<'s, JsKey>> {
        if self.peek() != Some(0x22) {
            return Err(self.error(cx));
        }
        let s = self.string(cx)?;
        self.expect(cx, 0x3A)?;
        Ok(cx.intern(s))
    }

    /// `true`, `false` or `null`.
    fn literal<'s>(
        &mut self,
        cx: &mut Cx<'s>,
        word: &str,
        value: Local<'s>,
    ) -> JsResult<Local<'s>> {
        for b in word.bytes() {
            if self.peek() != Some(u16::from(b)) {
                return Err(self.error(cx));
            }
            self.at += 1;
        }
        Ok(value)
    }

    /// A JSON number: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`, rounded to the
    /// nearest double (Rust's `str::parse::<f64>` is correctly rounded).
    fn number<'s>(&mut self, cx: &mut Cx<'s>) -> JsResult<f64> {
        let start = self.at;
        let digit = |u: Option<u16>| u.is_some_and(|u| (0x30..=0x39).contains(&u));
        if self.peek() == Some(0x2D) {
            self.at += 1;
        }
        match self.peek() {
            Some(0x30) => self.at += 1,
            Some(0x31..=0x39) => {
                while digit(self.peek()) {
                    self.at += 1;
                }
            }
            _ => return Err(self.error(cx)),
        }
        let int_end = self.at;
        if self.peek() == Some(0x2E) {
            self.at += 1;
            if !digit(self.peek()) {
                return Err(self.error(cx));
            }
            while digit(self.peek()) {
                self.at += 1;
            }
        }
        if matches!(self.peek(), Some(0x65 | 0x45)) {
            self.at += 1;
            if matches!(self.peek(), Some(0x2B | 0x2D)) {
                self.at += 1;
            }
            if !digit(self.peek()) {
                return Err(self.error(cx));
            }
            while digit(self.peek()) {
                self.at += 1;
            }
        }
        let units = &self.text[start..self.at];
        // A short integer is exact in an f64: no text round trip.
        if self.at == int_end && units.len() <= 15 {
            let (neg, digits) = match units {
                [0x2D, rest @ ..] => (true, rest),
                _ => (false, units),
            };
            let n = digits
                .iter()
                .fold(0u64, |n, &u| n * 10 + u64::from(u - 0x30)) as f64;
            return Ok(if neg { -n } else { n });
        }
        let text: String = units.iter().map(|&u| u as u8 as char).collect();
        Ok(text
            .parse::<f64>()
            .expect("a JSON number is a Rust float literal"))
    }

    /// A JSON string at `"`: its value as a new string.
    fn string<'s>(&mut self, cx: &mut Cx<'s>) -> JsResult<Local<'s, ep9_vm::JsString>> {
        debug_assert_eq!(self.peek(), Some(0x22));
        self.at += 1;
        let start = self.at;
        // The fast path: no escape, so the value is a slice of the text.
        loop {
            match self.peek() {
                None => return Err(self.unterminated(cx)),
                Some(0x22) => {
                    let s = cx.new_string_utf16(&self.text[start..self.at])?;
                    self.at += 1;
                    return Ok(s);
                }
                Some(0x5C) => break,
                Some(u) if u < 0x20 => return Err(self.bad_control(cx)),
                Some(_) => self.at += 1,
            }
        }
        let mut out: Vec<u16> = self.text[start..self.at].to_vec();
        loop {
            match self.peek() {
                None => return Err(self.unterminated(cx)),
                Some(0x22) => {
                    self.at += 1;
                    return cx.new_string_utf16(&out);
                }
                Some(0x5C) => {
                    self.at += 1;
                    let u = match self.peek() {
                        Some(0x22) => 0x22,
                        Some(0x5C) => 0x5C,
                        Some(0x2F) => 0x2F,
                        Some(0x62) => 0x08,
                        Some(0x66) => 0x0C,
                        Some(0x6E) => 0x0A,
                        Some(0x72) => 0x0D,
                        Some(0x74) => 0x09,
                        Some(0x75) => {
                            let mut u = 0u16;
                            for k in 1..=4 {
                                let d = self
                                    .text
                                    .get(self.at + k)
                                    .and_then(|&h| char::from_u32(u32::from(h)))
                                    .and_then(|c| c.to_digit(16));
                                let Some(d) = d else {
                                    self.at += k;
                                    return Err(self.bad_escape(cx));
                                };
                                u = u * 16 + d as u16;
                            }
                            self.at += 4;
                            u
                        }
                        _ => return Err(self.bad_escape(cx)),
                    };
                    self.at += 1;
                    out.push(u);
                }
                Some(u) if u < 0x20 => return Err(self.bad_control(cx)),
                Some(u) => {
                    self.at += 1;
                    out.push(u);
                }
            }
        }
    }

    fn unterminated<'s>(&self, cx: &mut Cx<'s>) -> Throw {
        let msg = format!("Unterminated string in JSON at position {}", self.at);
        cx.throw_error(ErrorKind::SyntaxError, &msg)
    }

    fn bad_control<'s>(&self, cx: &mut Cx<'s>) -> Throw {
        let msg = format!(
            "Bad control character in string literal in JSON at position {}",
            self.at
        );
        cx.throw_error(ErrorKind::SyntaxError, &msg)
    }

    fn bad_escape<'s>(&self, cx: &mut Cx<'s>) -> Throw {
        let msg = format!("Bad escaped character in JSON at position {}", self.at);
        cx.throw_error(ErrorKind::SyntaxError, &msg)
    }
}
