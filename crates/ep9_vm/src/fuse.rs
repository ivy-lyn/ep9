// The registry API has no caller outside `tests::fuse` until the first compiled reader (the
// link step of a unit, H35; JIT code) and the first wired builtin pop site come.
#![cfg_attr(not(test), allow(dead_code))]
//! The fuse registry v0 (M5-P1 FR0; CONSTREP appends descriptor kind 3; H31, C-01,
//! MC-w16-15, MC-w15-9, V10-D12): named facts that code depends on. A fuse is intact until a
//! change pops it; a pop is one-way and invalidates every dependent code handle.
//!
//! ```text
//!  FuseRef::Static(i)        a builtin fuse, by stable index ([`STATIC`]; H13 list)
//!  FuseRef::Dynamic(d)       a fuse named by a descriptor ([`Descriptor`]):
//!    kind 1 ProtoChain(ProtoRef)              the chain from that prototype up to null
//!    kind 2 ConstructorChain(FunctionRef)     the chain from that function up to null
//!    kind 3 field constness and representation (CONSTREP appends it)
//!
//!  Registry (Runtime::fuses, per isolate; Rust memory, no Value):
//!    static:  [StaticFuse; 11]   word, wired bit, dependents (Weak<code handle>)
//!    armed:   Descriptor -> DynFuse { word, subjects [(weak subject, strong VALIDITY_CELL)],
//!                                     dependents (Weak<code handle>) }
//!    popped:  Descriptor set (one-way: a popped descriptor never arms again)
//!
//!  arm(rt, d, subject)  check the name and the chain (no proxy), prototype mode, cell_for
//!  depend(f, code)      code depends on f (an InvalidatingFuse)
//!  guard(f) / intact(f) the GuardFuse read: one load + compare of the fuse word
//!  pop(rt, f)           word := POPPED, every live dependent Invalidated, handles released
//!  shape::note_change(o) on a prototype -> validity::invalidate -> on_proto_change
//!                       -> pop every armed fuse whose subject cell is now invalid
//! ```
//!
//! Rules:
//! - One-way (H31 Rules). A popped word never reads intact again. A popped descriptor is
//!   refused by [`Registry::arm`] and [`Registry::depend`] for the life of the isolate.
//! - A dynamic fuse exists from its first arm. It guards the chains of the subjects it was
//!   armed with, from the arm on: a chain change before the arm is not a pop. A pop of a
//!   descriptor that is not armed changes nothing.
//! - Prototype-chain validity reuses validity cells (H16 K5; `validity.rs`): a subject's cell
//!   guards its whole chain to null, and `shape::note_change` invalidates it on a structural
//!   change of any object on that chain. The registry holds the subject's cell by a strong
//!   handle (a cell refers to no prototype, so it keeps no object alive) and the subject by a
//!   weak handle. The descriptor names the prototype, never the cell: an invalid cell stays
//!   invalid and the next IC miss makes a new one (doc 13 §3.5, MC-w13-11).
//! - A chain that holds a proxy is refused (MC-w16-15): a proxy's `[[GetPrototypeOf]]` is a
//!   trap, so its chain can change with no structural change. So is a chain deeper than
//!   `validity::MAX_DEPTH`, and a subject that the descriptor does not name.
//! - Scope (doc 13 §3.5, F15). A fuse is per isolate: one word and one dependents list per
//!   descriptor, so a pop through any armed subject (any realm) invalidates every dependent.
//!   A descriptor names an object per realm (an intrinsic), or per closure (a class
//!   prototype), so code that runs against another subject of the same name arms the fuse
//!   with that subject first, or reads it as a GuardFuse there.
//! - Static fuses are named by index in the H31 Decision order. A static fuse is usable
//!   ([`Registry::depend`], [`Registry::guard`], an intact read) only once its pop site is
//!   wired ([`Registry::wire`]); v0 wires none, so each reads not intact.
//! - Dependents are weak code handles ([`CodeHandle`]): the registry holds `Weak`s and drops
//!   the dead ones. A pop marks each live one Invalidated; the lazy deopt of its frames and
//!   the dispatch-slot swap come with H29 lazy deopt (month 6).
//! - GC (plan rule 8): the registry holds handle-table indices, `Rc` fuse words and `Weak`
//!   code handles, never a Value. [`on_proto_change`] and [`Registry::pop`] are not GC points;
//!   [`Registry::arm`] is one.
//!
//! The FUSE kind id 83 stays reserved (OBJECT_LAYOUT 5): v0 needs no heap cell (H31-Q1, Q3:
//! the fuse word is Rust memory until a unit link cell or a root table names it).

use core::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use ep9_facts::{FunctionRef, ProtoRef};
use ep9_heap::{GlobalHandle, WeakHandle};

use crate::atoms::Atom;
use crate::builtins::{self, BuiltinId, IntrinsicId};
use crate::frame::NULL;
use crate::kinds::{self as kind, is_cell_bits};
use crate::object::{self, attr};
use crate::props;
use crate::runtime::{Runtime, val};
use crate::shape;
use crate::validity;

/// The descriptor kinds (H31 Freeze table "Dynamic descriptor kinds"; append-only).
pub(crate) mod descriptor {
    /// Prototype-chain validity by ProtoRef (C-01, MC-w16-15).
    pub const PROTO_CHAIN: u8 = 1;
    /// The constructor chain by FunctionRef (MC-w15-9, V10-D12).
    pub const CONSTRUCTOR_CHAIN: u8 = 2;

    /// True for a kind this build knows. A decoder (a unit's link table, H35) refuses the
    /// others, and the variant that names one is skipped (H31 Freeze table).
    pub fn known(k: u8) -> bool {
        matches!(k, PROTO_CHAIN | CONSTRUCTOR_CHAIN)
    }
}

/// A dynamic fuse name: a descriptor kind and its subject.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Descriptor {
    /// Kind 1: no object on the chain from this prototype up to null has a structural change
    /// (`shape::note_change`). `ProtoRef::Unknown` names nothing and is refused.
    ProtoChain(ProtoRef),
    /// Kind 2: no object on the chain from this function (the function itself, then its
    /// `[[Prototype]]`, up to null) has a structural change. The guard of `x instanceof D`
    /// and of static loads on a derived class (MC-w15-9), which no ProtoRef can name.
    ConstructorChain(FunctionRef),
}

impl Descriptor {
    /// The descriptor kind ([`descriptor`]).
    pub(crate) fn kind(&self) -> u8 {
        match self {
            Descriptor::ProtoChain(_) => descriptor::PROTO_CHAIN,
            Descriptor::ConstructorChain(_) => descriptor::CONSTRUCTOR_CHAIN,
        }
    }
}

/// The name of a fuse (H31-Q1, C-01).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FuseRef {
    /// A builtin fuse by stable index ([`STATIC`]).
    Static(u32),
    /// A fuse named by a descriptor.
    Dynamic(Descriptor),
}

/// The static fuses, by stable index: the builtins and debugger rows of the H31 Decision
/// list, in its order (the iterators entry is four fuses). Append-only (H31 Freeze table,
/// H13 index list): an index never changes.
pub(crate) const STATIC: [&str; 11] = [
    "Array species",
    "Array iterator",
    "String iterator",
    "Map iterator",
    "Set iterator",
    "Promise.then lookup",
    "no-elements prototypes",
    "RegExp exec",
    "TypedArray species",
    "InspectorSession",
    "DebuggerBindingWrite",
];

/// Why the registry refuses a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A static index past [`STATIC`].
    UnknownStatic,
    /// A static fuse whose pop site is not wired: nothing would pop it.
    NotWired,
    /// The fuse popped (one-way).
    Popped,
    /// A dynamic fuse that is not armed: [`Registry::arm`] it with its subject first.
    NotArmed,
    /// The descriptor does not name the subject (or names nothing: `ProtoRef::Unknown`).
    NotNamed,
    /// A proxy on the chain (MC-w16-15).
    ProxyOnChain,
    /// The chain is deeper than `validity::MAX_DEPTH`: no validity cell guards it.
    TooDeep,
}

/// The fuse word: what a GuardFuse reads (H31-Q3; v0 Rust memory). One load + compare.
#[derive(Debug)]
pub(crate) struct FuseWord(Cell<u32>);

/// The word of an intact fuse.
const INTACT: u32 = 0;
/// The word of a popped fuse.
const POPPED: u32 = 1;

impl FuseWord {
    fn new() -> Rc<FuseWord> {
        Rc::new(FuseWord(Cell::new(INTACT)))
    }

    /// The GuardFuse read: true while the fuse is intact.
    #[inline(always)]
    pub(crate) fn intact(&self) -> bool {
        self.0.get() == INTACT
    }
}

/// A code object's dependency handle (an InvalidatingFuse dependent). The code owns it; the
/// registry holds it weakly, so a dropped code object leaves only a dead `Weak`, which the
/// registry drops.
#[derive(Clone, Debug, Default)]
pub(crate) struct CodeHandle(Rc<CodeState>);

#[derive(Debug, Default)]
struct CodeState {
    invalidated: Cell<bool>,
}

impl CodeHandle {
    /// A handle for a new code object (not Invalidated).
    pub(crate) fn new() -> CodeHandle {
        CodeHandle::default()
    }

    /// True once a fuse this code depends on popped.
    pub(crate) fn is_invalidated(&self) -> bool {
        self.0.invalidated.get()
    }

    fn weak(&self) -> Weak<CodeState> {
        Rc::downgrade(&self.0)
    }
}

/// A dependents list: weak code handles.
#[derive(Default)]
struct Dependents(Vec<Weak<CodeState>>);

impl Dependents {
    /// Adds `c` (once). A full list first drops its dead entries, so it holds at most twice
    /// the live ones.
    fn add(&mut self, c: &CodeHandle) {
        let w = c.weak();
        if self.0.iter().any(|d| Weak::ptr_eq(d, &w)) {
            return;
        }
        if self.0.len() == self.0.capacity() {
            self.sweep();
        }
        self.0.push(w);
    }

    fn sweep(&mut self) {
        self.0.retain(|d| d.strong_count() > 0);
    }

    /// Marks every live dependent Invalidated and empties the list.
    fn invalidate_all(&mut self) {
        for d in self.0.drain(..) {
            if let Some(c) = d.upgrade() {
                c.invalidated.set(true);
            }
        }
    }
}

struct StaticFuse {
    word: Rc<FuseWord>,
    wired: bool,
    dependents: Dependents,
}

/// One armed subject of a dynamic fuse.
struct Subject {
    /// The subject object (weak; `None` for the null prototype).
    object: Option<WeakHandle>,
    /// Its VALIDITY_CELL at the arm (strong).
    cell: GlobalHandle,
}

struct DynFuse {
    word: Rc<FuseWord>,
    subjects: Vec<Subject>,
    dependents: Dependents,
}

struct Inner {
    statics: Vec<StaticFuse>,
    armed: HashMap<Descriptor, DynFuse>,
    popped: HashSet<Descriptor>,
}

/// The per-isolate fuse registry (`Runtime::fuses`).
pub(crate) struct Registry {
    inner: RefCell<Inner>,
    /// The number of armed dynamic fuses: [`on_proto_change`] reads only this while it is 0.
    armed: Cell<usize>,
}

impl Registry {
    /// The registry of a new isolate: every static fuse intact and not wired, no dynamic
    /// fuse armed.
    pub(crate) fn new() -> Registry {
        let statics = STATIC
            .iter()
            .map(|_| StaticFuse {
                word: FuseWord::new(),
                wired: false,
                dependents: Dependents::default(),
            })
            .collect();
        Registry {
            inner: RefCell::new(Inner {
                statics,
                armed: HashMap::new(),
                popped: HashSet::new(),
            }),
            armed: Cell::new(0),
        }
    }

    /// True if no dynamic fuse is armed and no static fuse has a dependent.
    pub(crate) fn is_empty(&self) -> bool {
        let inner = self.inner.borrow();
        inner.armed.is_empty() && inner.statics.iter().all(|s| s.dependents.0.is_empty())
    }

    /// Marks the static fuse `i` wired: its pop site exists, so code may use it. The owner of
    /// the pop site calls it when the isolate starts.
    pub(crate) fn wire(&self, i: u32) -> Result<(), Refusal> {
        let mut inner = self.inner.borrow_mut();
        let s = inner
            .statics
            .get_mut(i as usize)
            .ok_or(Refusal::UnknownStatic)?;
        s.wired = true;
        Ok(())
    }

    /// Arms the dynamic fuse `d` with `subject`: the object `d` names in the current realm
    /// (null for `ProtoChain(Null)`). From now on a structural change of an object on the
    /// subject's chain pops `d`. Gives the fuse word. Arming an armed fuse with another
    /// subject adds that subject; with the same one it changes nothing. Refused: a popped
    /// descriptor, a subject the descriptor does not name, a proxy on the chain, a chain
    /// deeper than `validity::MAX_DEPTH`. Puts the chain's first object in prototype mode
    /// (`shape::proto_info_for`). GC point.
    pub(crate) fn arm(
        &self,
        rt: &Runtime,
        d: Descriptor,
        subject: u64,
    ) -> Result<Rc<FuseWord>, Refusal> {
        if self.inner.borrow().popped.contains(&d) {
            return Err(Refusal::Popped);
        }
        if !names(rt, &d, subject) {
            return Err(Refusal::NotNamed);
        }
        check_chain(subject)?;
        // GC points below: `subject` lives in a Rust local (CSS); no JS runs, so the chain
        // stays the one checked above.
        if subject != NULL {
            shape::proto_info_for(rt, subject);
        }
        let c = validity::cell_for(rt, subject).ok_or(Refusal::TooDeep)?;
        let mut inner = self.inner.borrow_mut();
        debug_assert!(!inner.popped.contains(&d), "a GC pops nothing");
        let fresh = !inner.armed.contains_key(&d);
        let f = inner.armed.entry(d).or_insert_with(|| DynFuse {
            word: FuseWord::new(),
            subjects: Vec::new(),
            dependents: Dependents::default(),
        });
        drop_dead_subjects(rt, &mut f.subjects);
        if !f
            .subjects
            .iter()
            .any(|s| rt.heap.global(s.cell).to_bits() == c)
        {
            f.subjects.push(Subject {
                object: (subject != NULL).then(|| rt.heap.new_weak(val(subject))),
                cell: rt.heap.new_global(val(c)),
            });
        }
        let word = Rc::clone(&f.word);
        if fresh {
            self.armed.set(self.armed.get() + 1);
        }
        Ok(word)
    }

    /// Makes `code` depend on `f` (an InvalidatingFuse): a pop of `f` marks it Invalidated.
    /// A static fuse must be wired and a dynamic one armed. Not a GC point.
    pub(crate) fn depend(&self, f: FuseRef, code: &CodeHandle) -> Result<(), Refusal> {
        let mut inner = self.inner.borrow_mut();
        match f {
            FuseRef::Static(i) => {
                let s = inner
                    .statics
                    .get_mut(i as usize)
                    .ok_or(Refusal::UnknownStatic)?;
                if !s.wired {
                    return Err(Refusal::NotWired);
                }
                if !s.word.intact() {
                    return Err(Refusal::Popped);
                }
                s.dependents.add(code);
            }
            FuseRef::Dynamic(d) => {
                if inner.popped.contains(&d) {
                    return Err(Refusal::Popped);
                }
                let f = inner.armed.get_mut(&d).ok_or(Refusal::NotArmed)?;
                f.dependents.add(code);
            }
        }
        Ok(())
    }

    /// The word of `f` for a GuardFuse reader (a Rust fast path, H31-Q5): a wired static fuse
    /// or an armed dynamic one. The word reads intact until `f` pops. Not a GC point.
    pub(crate) fn guard(&self, f: FuseRef) -> Result<Rc<FuseWord>, Refusal> {
        let inner = self.inner.borrow();
        match f {
            FuseRef::Static(i) => {
                let s = inner
                    .statics
                    .get(i as usize)
                    .ok_or(Refusal::UnknownStatic)?;
                if s.wired {
                    Ok(Rc::clone(&s.word))
                } else {
                    Err(Refusal::NotWired)
                }
            }
            FuseRef::Dynamic(d) => {
                if inner.popped.contains(&d) {
                    return Err(Refusal::Popped);
                }
                inner
                    .armed
                    .get(&d)
                    .map(|f| Rc::clone(&f.word))
                    .ok_or(Refusal::NotArmed)
            }
        }
    }

    /// True if `f` is intact: a wired static fuse or an armed dynamic one that has not
    /// popped. Anything else reads false, so a reader takes its slow path. Not a GC point.
    pub(crate) fn intact(&self, f: FuseRef) -> bool {
        self.guard(f).is_ok_and(|w| w.intact())
    }

    /// Pops `f`: its word reads POPPED from now on, every live dependent is Invalidated, and
    /// a dynamic fuse releases its handles and is never armed again. A popped fuse, or a
    /// dynamic one that is not armed, changes nothing. Not a GC point.
    pub(crate) fn pop(&self, rt: &Runtime, f: FuseRef) {
        let mut inner = self.inner.borrow_mut();
        match f {
            FuseRef::Static(i) => {
                if let Some(s) = inner.statics.get_mut(i as usize) {
                    s.word.0.set(POPPED);
                    s.dependents.invalidate_all();
                }
            }
            FuseRef::Dynamic(d) => {
                if let Some(mut f) = inner.armed.remove(&d) {
                    self.armed.set(self.armed.get() - 1);
                    f.word.0.set(POPPED);
                    f.dependents.invalidate_all();
                    release_subjects(rt, f.subjects.drain(..));
                    inner.popped.insert(d);
                }
            }
        }
    }

    /// Drops dead dependents and dead subjects, and the armed fuses that nothing uses (no
    /// live dependent and no word held outside the registry). Not a GC point.
    pub(crate) fn sweep(&self, rt: &Runtime) {
        let mut inner = self.inner.borrow_mut();
        for s in &mut inner.statics {
            s.dependents.sweep();
        }
        let mut unused = Vec::new();
        for (d, f) in &mut inner.armed {
            f.dependents.sweep();
            drop_dead_subjects(rt, &mut f.subjects);
            if f.dependents.0.is_empty() && Rc::strong_count(&f.word) == 1 {
                unused.push(*d);
            }
        }
        for d in unused {
            let f = inner.armed.remove(&d).expect("listed above");
            release_subjects(rt, f.subjects.into_iter());
            self.armed.set(self.armed.get() - 1);
        }
    }

    /// The entries of the dependents list of `f` (dead ones not yet dropped included), or 0.
    pub(crate) fn dependent_entries(&self, f: FuseRef) -> usize {
        let inner = self.inner.borrow();
        match f {
            FuseRef::Static(i) => inner
                .statics
                .get(i as usize)
                .map_or(0, |s| s.dependents.0.len()),
            FuseRef::Dynamic(d) => inner.armed.get(&d).map_or(0, |f| f.dependents.0.len()),
        }
    }

    /// The subjects a dynamic fuse is armed with (dead ones not yet dropped included), or 0.
    pub(crate) fn subject_count(&self, d: Descriptor) -> usize {
        self.inner
            .borrow()
            .armed
            .get(&d)
            .map_or(0, |f| f.subjects.len())
    }

    /// The armed dynamic fuses whose subject cell is no longer valid.
    #[cold]
    #[inline(never)]
    fn changed(&self, rt: &Runtime) -> Vec<Descriptor> {
        let inner = self.inner.borrow();
        inner
            .armed
            .iter()
            .filter(|(_, f)| {
                f.subjects
                    .iter()
                    .any(|s| !validity::is_valid(rt.heap.global(s.cell).to_bits()))
            })
            .map(|(d, _)| *d)
            .collect()
    }
}

/// Releases the handles of `subjects`.
fn release_subjects(rt: &Runtime, subjects: impl Iterator<Item = Subject>) {
    for s in subjects {
        if let Some(o) = s.object {
            rt.heap.drop_weak(o);
        }
        rt.heap.drop_global(s.cell);
    }
}

/// Drops the subjects whose object died (a GC cleared the weak handle).
fn drop_dead_subjects(rt: &Runtime, subjects: &mut Vec<Subject>) {
    let (live, dead): (Vec<Subject>, Vec<Subject>) = subjects
        .drain(..)
        .partition(|s| s.object.is_none_or(|o| rt.heap.weak(o).is_some()));
    *subjects = live;
    release_subjects(rt, dead.into_iter());
}

/// True if `d` names `s` in the current realm. Not a GC point.
fn names(rt: &Runtime, d: &Descriptor, s: u64) -> bool {
    match *d {
        Descriptor::ProtoChain(ProtoRef::Null) => s == NULL,
        Descriptor::ProtoChain(ProtoRef::Intrinsic(i)) => {
            kind::is_object(s)
                && IntrinsicId::ALL
                    .get(i as usize)
                    .is_some_and(|&id| id as u32 == i && builtins::intrinsic(rt, id) == s)
        }
        Descriptor::ProtoChain(ProtoRef::FunctionPrototype(r)) => {
            kind::is_object(s) && class_of_prototype(rt, s).is_some_and(|f| named(rt, &r, f))
        }
        Descriptor::ProtoChain(ProtoRef::Unknown) => false,
        Descriptor::ConstructorChain(r) => named(rt, &r, s),
    }
}

/// The class constructor `F` whose initial prototype `p` is, with the proof the VM keeps
/// (the `facts_export.rs` `function_prototype` test, Codex C-FX2): `p`'s own data property
/// `constructor` is a class constructor `F` whose `[[HomeObject]]` is `p` and whose own
/// `prototype` is `p`, neither writable nor configurable. Not a GC point.
fn class_of_prototype(rt: &Runtime, p: u64) -> Option<u64> {
    let (f, at) = props::own_get(p, rt.atom(Atom::Constructor))?;
    if at & attr::ACCESSOR != 0 || !kind::is_function(f) {
        return None;
    }
    if !object::function_info(f).is_class_constructor() || object::home_object(f) != p {
        return None;
    }
    let (proto, at) = props::own_get(f, rt.atom(Atom::Prototype))?;
    (proto == p && at & (attr::ACCESSOR | attr::WRITABLE | attr::CONFIGURABLE) == 0).then_some(f)
}

/// True if `r` names the FUNCTION cell `f`: a unit function by unit content hash and
/// function id (never function 0, H02), or a table builtin of the current realm. Not a GC
/// point.
fn named(rt: &Runtime, r: &FunctionRef, f: u64) -> bool {
    if !is_cell_bits(f) || !kind::is_function(f) {
        return false;
    }
    let fi = object::function_info(f);
    match *r {
        FunctionRef::Js { unit, function } => {
            function != 0
                && fi.function_id == function
                && fi
                    .loaded_unit()
                    .is_some_and(|u| u.unit.content_hash() == unit)
        }
        FunctionRef::Builtin(bits) => {
            let id = BuiltinId::new(
                (bits >> BuiltinId::INDEX_BITS) as u8,
                bits & (BuiltinId::PER_BLOCK as u16 - 1),
            );
            builtins::existing_function(rt, id) == Some(f)
        }
    }
}

/// Refuses a chain from `s` (the subject, or null) that holds a proxy or is deeper than
/// `validity::MAX_DEPTH`. Not a GC point.
fn check_chain(s: u64) -> Result<(), Refusal> {
    let mut q = s;
    let mut depth = 0;
    while q != NULL {
        if crate::proxy::is_proxy(q) {
            return Err(Refusal::ProxyOnChain);
        }
        depth += 1;
        if depth > validity::MAX_DEPTH {
            return Err(Refusal::TooDeep);
        }
        q = object::proto_of(q);
    }
    Ok(())
}

/// The hook of a structural change of the prototype object `o` (`shape::note_change`, after
/// the validity cells are invalidated): pops every armed dynamic fuse whose subject cell
/// is no longer valid. One load and one branch while no dynamic fuse is armed. Not a GC
/// point.
#[inline(always)]
pub(crate) fn on_proto_change(rt: &Runtime, _o: u64) {
    if rt.fuses.armed.get() != 0 {
        pop_changed(rt);
    }
}

#[cold]
#[inline(never)]
fn pop_changed(rt: &Runtime) {
    for d in rt.fuses.changed(rt) {
        rt.fuses.pop(rt, FuseRef::Dynamic(d));
    }
}
