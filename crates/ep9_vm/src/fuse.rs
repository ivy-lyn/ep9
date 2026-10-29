// Most of the registry API has no caller outside `tests::fuse` until the first compiled reader
// (the link step of a unit, H35; JIT code) and the first wired builtin pop site come (the
// runtime callers today: `shape::note_change` and the cleanup point's sweep).
#![cfg_attr(not(test), allow(dead_code))]
//! The fuse registry v0 (M5-P1 FR0; M5-P2 CONSTREP: descriptor kind 3 and one word per armed
//! subject; H31, C-01, MC-w16-15, MC-w15-9, V10-D12, M5-P1-ORCH-13): named facts that code
//! depends on. A fuse is intact until a change pops it; a pop is one-way and invalidates every
//! dependent code handle.
//!
//! ```text
//!  FuseRef::Static(i)        a builtin fuse, by stable index ([`STATIC`]; H13 list)
//!  FuseRef::Dynamic(d)       a fuse named by a descriptor ([`Descriptor`]) and armed per subject:
//!    kind 1 ProtoChain(ProtoRef)              subject: that prototype (per realm); the chain
//!                                             from it up to null
//!    kind 2 ConstructorChain(FunctionRef)     subject: that function; the chain from it up
//!    kind 3 Field { index }                   subject: a fast shape (run-local); the
//!                                             representation and const bits of descriptor
//!                                             `index` of its field family (M5-P2 CONSTREP)
//!
//!  Registry (Runtime::fuses, per isolate; Rust memory, no Value):
//!    static:  [StaticFuse; 11]   word, wired bit, dependents (Weak<code handle>)
//!    armed:   Descriptor -> [Subject { weak subject, kind 1-2: strong VALIDITY_CELL,
//!                                      kind 3: the field bits at the arm,
//!                                      word, dependents (Weak<code handle>) }]
//!    popped:  [(Descriptor, weak subject)]   one-way: that subject never arms again
//!    popped_any: Descriptor set   kinds 1-2: popped in some realm; blocks depend
//!
//!  arm(rt, d, subject)  check the name (and the chain: no proxy), prototype mode, cell_for;
//!                       a new subject gets its own word
//!  depend(rt, f, subject, code)   code that runs against subject depends on f
//!                                 (an InvalidatingFuse)
//!  guard(rt, f, subject) / intact(rt, f, subject)
//!                       the GuardFuse read: one load + compare of the subject's word
//!  pop of a subject     its word := POPPED; kinds 1-2: every dependent of the descriptor (in
//!                       every realm) Invalidated, popped_any; kind 3: the subject's dependents
//!  sweep_after_gc(rt)   at the cleanup point after a GC (Cx::cleanup_finalization_registries):
//!                       drop dead dependents, dead subjects and the subjects nothing uses
//!  shape::note_change(o) on a prototype -> validity::invalidate -> on_proto_change
//!                       -> pop every armed kind-1/2 subject whose cell is now invalid
//!  field_rep widening of descriptor i -> on_field_change(i)
//!                       -> pop every armed kind-3 subject whose field bits changed
//! ```
//!
//! Rules:
//! - One-way (H31 Rules). A popped word never reads intact again, and its subject is refused
//!   by [`Registry::arm`] and [`Registry::guard`] for the life of the isolate (the popped list
//!   holds the subject weakly; a dead one needs no entry).
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
//! - Scope (H31-Q2, M5-P1-ORCH-13). GuardFuse words are per armed subject: one word per
//!   (descriptor, subject), so a subject of another realm (another realm's intrinsic, another
//!   closure's class prototype) is armed on its own and keeps its own word. InvalidatingFuse
//!   dependents of kinds 1-2 are per isolate: a FunctionInfo and its code set are shared by
//!   realms (H22), so a pop of any subject invalidates the dependents of every subject of the
//!   descriptor and puts the descriptor in the "popped in any realm" summary, which refuses
//!   [`Registry::depend`] from then on; the other realms' words stay intact. A kind-3
//!   descriptor names no realm-wide object (a shape is run-local), so its dependents and its
//!   popped state are its subject's only. [`Registry::depend`], [`Registry::guard`] and
//!   [`Registry::intact`] take the caller's subject and refuse ([`Refusal::NotArmed`]) one that
//!   no arm names or whose state changed (kinds 1-2: its current validity cell is not the one
//!   the arm kept; kind 3: its field bits are not the ones the arm read) (M5-P1 review, Codex
//!   r1 fr0). The realm fuse block for static fuses (`Root::RealmFuses`) waits for the first
//!   wired static fuse (month 6, H31-Q5).
//! - Kind 3 (M5-P2 CONSTREP, C-02): the subject is any fast shape that has descriptor
//!   `index` (not an accessor); the arm keeps its field owner (`shape::field_owner`), so every
//!   shape of the family names one fuse. A widening of the field's bits (`field_rep.rs`) pops
//!   it. The word a compiled reader would load is reached through the shape: SHAPE +24
//!   DESCRIPTORS, the entry's details bits 4-7 (H17 rep/const row); the Rust word mirrors it.
//! - Subject forms are an append-only list (H31 Rules): ProtoRef (kind 1), FunctionRef (kind
//!   2), a run-local field: a shape and a descriptor index (kind 3; no unit or snapshot names
//!   one, R17).
//! - Static fuses are named by index in the H31 Decision order. A static fuse is usable
//!   ([`Registry::depend`], [`Registry::guard`], an intact read) only once its pop site is
//!   wired ([`Registry::wire`]); v0 wires none, so each reads not intact.
//! - Dependents are weak code handles ([`CodeHandle`]): the registry holds `Weak`s and drops
//!   the dead ones. A pop marks each live one Invalidated; the lazy deopt of its frames and
//!   the dispatch-slot swap come with H29 lazy deopt (month 6).
//! - GC (plan rule 8): the registry holds handle-table indices, `Rc` fuse words, int32 field
//!   bits and `Weak` code handles, never a Value. [`on_proto_change`], [`on_field_change`] and
//!   [`Registry::pop`] are not GC points; [`Registry::arm`] of kinds 1-2 is one. Subjects are
//!   compared by the current value of their weak handles, by equality only (H12: never a
//!   hash or a sort key).
//! - The runtime sweep: a weak source notes each GC (it cannot sweep there: the heap's handle
//!   table is borrowed while the weak sources run). The next FinalizationRegistry cleanup
//!   point (`Cx::cleanup_finalization_registries`, which every microtask checkpoint runs)
//!   calls [`Registry::sweep_after_gc`]: dead dependents and dead subjects go, and so does an
//!   armed subject that nothing uses (no live dependent and no word held outside the
//!   registry), with its handles (M5-P1 fixer round 3, Codex r1 fr0).
//!
//! The FUSE kind id 83 stays reserved (OBJECT_LAYOUT 5): v0 needs no heap cell (H31-Q1, Q3:
//! the fuse word is Rust memory until a unit link cell or a root table names it).

use core::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use ep9_facts::{FunctionRef, ProtoRef};
use ep9_heap::{GlobalHandle, Heap, WeakHandle};

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
    /// Field constness and representation (C-02, M5-P2 CONSTREP): a run-local field (a shape
    /// and a descriptor index).
    pub const FIELD: u8 = 3;

    /// True for a kind this build knows. A decoder (a unit's link table, H35) refuses the
    /// others, and the variant that names one is skipped (H31 Freeze table).
    pub fn known(k: u8) -> bool {
        matches!(k, PROTO_CHAIN | CONSTRUCTOR_CHAIN | FIELD)
    }
}

/// A dynamic fuse name: a descriptor kind and its subject form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Descriptor {
    /// Kind 1: no object on the chain from this prototype up to null has a structural change
    /// (`shape::note_change`). `ProtoRef::Unknown` names nothing and is refused.
    ProtoChain(ProtoRef),
    /// Kind 2: no object on the chain from this function (the function itself, then its
    /// `[[Prototype]]`, up to null) has a structural change. The guard of `x instanceof D`
    /// and of static loads on a derived class (MC-w15-9), which no ProtoRef can name.
    ConstructorChain(FunctionRef),
    /// Kind 3 (M5-P2 CONSTREP, C-02): the representation and const bits of descriptor `index`
    /// of a field family (`field_rep.rs`) do not change. The subject is a fast shape that has
    /// the field; a shape is run-local, so the subject is part of the fuse's identity (no
    /// unit or snapshot names it, R17).
    Field { index: u16 },
}

impl Descriptor {
    /// The descriptor kind ([`descriptor`]).
    pub(crate) fn kind(&self) -> u8 {
        match self {
            Descriptor::ProtoChain(_) => descriptor::PROTO_CHAIN,
            Descriptor::ConstructorChain(_) => descriptor::CONSTRUCTOR_CHAIN,
            Descriptor::Field { .. } => descriptor::FIELD,
        }
    }

    /// True if the descriptor names one object per realm (kinds 1-2), so its dependents are
    /// per isolate: a pop of any subject invalidates them all (H31-Q2, M5-P1-ORCH-13).
    fn realm_wide(&self) -> bool {
        !matches!(self, Descriptor::Field { .. })
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
    /// The fuse popped (one-way): for this subject, or (depend, kinds 1-2) in some realm.
    Popped,
    /// A dynamic fuse that is not armed with this subject: [`Registry::arm`] it first.
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

/// What a subject's state is checked against: kinds 1-2 its VALIDITY_CELL at the arm
/// (strong), kind 3 its field bits at the arm (an int32, `shape::FIELD_BITS`).
enum Watch {
    Cell(GlobalHandle),
    FieldBits(i32),
}

/// One armed subject of a dynamic fuse, with its own word (H31-Q2, M5-P1-ORCH-13).
struct Subject {
    /// The subject (weak; `None` for the null prototype): kinds 1-2 the prototype or
    /// function, kind 3 the field owner shape.
    object: Option<WeakHandle>,
    watch: Watch,
    word: Rc<FuseWord>,
    dependents: Dependents,
}

impl Subject {
    /// True if this entry is for the subject `id` (null, or the bits of a live cell): its
    /// weak handle reads `id`. An equality test only (H12). Not a GC point.
    fn is(&self, rt: &Runtime, id: u64) -> bool {
        match self.object {
            None => id == NULL,
            Some(o) => rt.heap.weak(o).is_some_and(|v| v.to_bits() == id),
        }
    }

    /// True if the state the arm saw still holds: kinds 1-2 the subject's current valid
    /// VALIDITY_CELL (no new one is made) is the kept one; kind 3 the field bits of descriptor
    /// `index` of the owner `id` are the kept ones. Not a GC point.
    fn current(&self, rt: &Runtime, id: u64, index: Option<usize>) -> bool {
        match (&self.watch, index) {
            (Watch::Cell(c), None) => {
                current_cell(rt, id).is_some_and(|now| rt.heap.global(*c).to_bits() == now)
            }
            (Watch::FieldBits(b), Some(i)) => shape::field_bits_at(id, i) == *b,
            _ => false,
        }
    }

    /// The subject's value (null, or the bits of a live cell), or `None` if it died. Not a GC
    /// point.
    fn id(&self, rt: &Runtime) -> Option<u64> {
        match self.object {
            None => Some(NULL),
            Some(o) => rt.heap.weak(o).map(|v| v.to_bits()),
        }
    }

    /// Releases the handles; gives the weak subject handle to keep (a popped subject).
    fn release(self, rt: &Runtime) -> Option<WeakHandle> {
        if let Watch::Cell(c) = self.watch {
            rt.heap.drop_global(c);
        }
        self.object
    }
}

/// The current valid VALIDITY_CELL of `subject` (null, or an object in prototype mode), or
/// `None`: any other value, an object not in prototype mode (never armed), or no valid cell
/// now (its chain changed since its cell was made). Makes no cell. Not a GC point.
fn current_cell(rt: &Runtime, subject: u64) -> Option<u64> {
    if subject != NULL
        && !(is_cell_bits(subject)
            && kind::is_object(subject)
            && shape::is_prototype_object(subject))
    {
        return None;
    }
    let c = shape::validity_cell_of(shape::proto_info_of(rt, subject));
    validity::is_valid(c).then_some(c)
}

/// A popped subject: refused by `arm` and `guard` for the life of the isolate.
struct Popped {
    d: Descriptor,
    /// Weak; `None` for the null prototype.
    object: Option<WeakHandle>,
}

struct Inner {
    statics: Vec<StaticFuse>,
    armed: HashMap<Descriptor, Vec<Subject>>,
    popped: Vec<Popped>,
    /// Kinds 1-2: descriptors popped in some realm (blocks `depend`).
    popped_any: HashSet<Descriptor>,
}

impl Inner {
    /// True if the subject `id` of `d` popped. Not a GC point.
    fn popped(&self, rt: &Runtime, d: Descriptor, id: u64) -> bool {
        self.popped.iter().any(|p| {
            p.d == d
                && match p.object {
                    None => id == NULL,
                    Some(o) => rt.heap.weak(o).is_some_and(|v| v.to_bits() == id),
                }
        })
    }

    /// The armed entry of the subject `id` of `d`, if its state still holds. Not a GC point.
    fn armed_with(&mut self, rt: &Runtime, d: Descriptor, id: u64) -> Option<&mut Subject> {
        let index = field_index(d);
        self.armed
            .get_mut(&d)?
            .iter_mut()
            .find(|s| s.is(rt, id) && s.current(rt, id, index))
    }
}

/// The descriptor index of a kind-3 descriptor.
fn field_index(d: Descriptor) -> Option<usize> {
    match d {
        Descriptor::Field { index } => Some(usize::from(index)),
        _ => None,
    }
}

/// The per-isolate fuse registry (`Runtime::fuses`).
pub(crate) struct Registry {
    inner: RefCell<Inner>,
    /// The number of armed kind-1/2 subjects: [`on_proto_change`] reads only this while it is
    /// 0.
    armed: Cell<usize>,
    /// The number of armed kind-3 subjects: [`on_field_change`] reads only this while it is 0.
    fields_armed: Cell<usize>,
    /// Set by the heap's weak source at every GC; cleared by [`Registry::sweep_after_gc`].
    gc_since_sweep: Rc<Cell<bool>>,
}

impl Registry {
    /// The registry of a new isolate: every static fuse intact and not wired, no dynamic
    /// fuse armed. Registers with `heap` a weak source that notes each GC (module doc: the
    /// runtime sweep). Not a GC point.
    pub(crate) fn new(heap: &Heap) -> Registry {
        let gc_since_sweep = Rc::new(Cell::new(false));
        let g = Rc::clone(&gc_since_sweep);
        heap.add_weak_source(move |_| g.set(true));
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
                popped: Vec::new(),
                popped_any: HashSet::new(),
            }),
            armed: Cell::new(0),
            fields_armed: Cell::new(0),
            gc_since_sweep,
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

    /// The counter of armed subjects of `d`'s kind.
    fn counter(&self, d: Descriptor) -> &Cell<usize> {
        if d.realm_wide() {
            &self.armed
        } else {
            &self.fields_armed
        }
    }

    /// Arms the dynamic fuse `d` with `subject` and gives the subject's word. Kinds 1-2: the
    /// object `d` names in the current realm (null for `ProtoChain(Null)`); from now on a
    /// structural change of an object on the subject's chain pops it. Kind 3: a fast shape
    /// that has descriptor `index` (a data property); the fuse is the field owner's, and a
    /// widening of its bits pops it. Arming an armed subject again gives its word. Refused: a
    /// popped subject (one-way), a subject the descriptor does not name, a proxy on the chain,
    /// a chain deeper than `validity::MAX_DEPTH`. Another realm's subject arms on its own,
    /// with its own word, even after a pop in this realm. Kinds 1-2 put the chain's first
    /// object in prototype mode (`shape::proto_info_for`): a GC point. Kind 3 is not one.
    pub(crate) fn arm(
        &self,
        rt: &Runtime,
        d: Descriptor,
        subject: u64,
    ) -> Result<Rc<FuseWord>, Refusal> {
        if !names(rt, &d, subject) {
            return Err(Refusal::NotNamed);
        }
        let id = subject_id(d, subject);
        if self.inner.borrow().popped(rt, d, id) {
            return Err(Refusal::Popped);
        }
        let watch = match field_index(d) {
            Some(i) => Watch::FieldBits(shape::field_bits_at(id, i)),
            None => {
                check_chain(subject)?;
                // GC points below: `subject` lives in a Rust local (CSS); no JS runs, so the
                // chain stays the one checked above.
                if subject != NULL {
                    shape::proto_info_for(rt, subject);
                }
                let c = validity::cell_for(rt, subject).ok_or(Refusal::TooDeep)?;
                Watch::Cell(rt.heap.new_global(val(c)))
            }
        };
        let mut inner = self.inner.borrow_mut();
        debug_assert!(!inner.popped(rt, d, id), "a GC pops nothing");
        let subjects = inner.armed.entry(d).or_default();
        drop_dead_subjects(rt, subjects, &self.armed, &self.fields_armed, d);
        let index = field_index(d);
        if let Some(at) = subjects.iter().position(|s| s.is(rt, id)) {
            if !subjects[at].current(rt, id, index) {
                // Its state changed, and the hook did not pop it: refuse and pop it now.
                drop(inner);
                if let Watch::Cell(c) = watch {
                    rt.heap.drop_global(c);
                }
                self.pop_subject(rt, d, id);
                return Err(Refusal::Popped);
            }
            if let Watch::Cell(c) = watch {
                rt.heap.drop_global(c);
            }
            return Ok(Rc::clone(&subjects[at].word));
        }
        let word = FuseWord::new();
        subjects.push(Subject {
            object: (id != NULL).then(|| rt.heap.new_weak(val(id))),
            watch,
            word: Rc::clone(&word),
            dependents: Dependents::default(),
        });
        let n = self.counter(d);
        n.set(n.get() + 1);
        Ok(word)
    }

    /// Makes `code` depend on `f` (an InvalidatingFuse): a pop of `f` marks it Invalidated. A
    /// static fuse must be wired (it ignores `subject`). A dynamic one must be armed with
    /// `subject`, the object (kind 3: the shape) the code runs against (null for
    /// `ProtoChain(Null)`), and its state must still hold ([`Refusal::NotArmed`] if not).
    /// Kinds 1-2 refuse once any subject of `d` popped, in any realm ([`Refusal::Popped`]).
    /// Not a GC point.
    pub(crate) fn depend(
        &self,
        rt: &Runtime,
        f: FuseRef,
        subject: u64,
        code: &CodeHandle,
    ) -> Result<(), Refusal> {
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
                if d.realm_wide() && inner.popped_any.contains(&d) {
                    return Err(Refusal::Popped);
                }
                let id = lookup_id(rt, d, subject).ok_or(Refusal::NotArmed)?;
                if inner.popped(rt, d, id) {
                    return Err(Refusal::Popped);
                }
                inner
                    .armed_with(rt, d, id)
                    .ok_or(Refusal::NotArmed)?
                    .dependents
                    .add(code);
            }
        }
        Ok(())
    }

    /// The word of `f` for a GuardFuse reader (a Rust fast path, H31-Q5): a wired static fuse
    /// (it ignores `subject`), or the word of the armed subject `subject` of a dynamic one (as
    /// [`Registry::depend`]; a pop in another realm leaves it intact). The word reads intact
    /// until that subject pops. Not a GC point.
    pub(crate) fn guard(
        &self,
        rt: &Runtime,
        f: FuseRef,
        subject: u64,
    ) -> Result<Rc<FuseWord>, Refusal> {
        let mut inner = self.inner.borrow_mut();
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
                let id = lookup_id(rt, d, subject).ok_or(Refusal::NotArmed)?;
                if inner.popped(rt, d, id) {
                    return Err(Refusal::Popped);
                }
                inner
                    .armed_with(rt, d, id)
                    .map(|s| Rc::clone(&s.word))
                    .ok_or(Refusal::NotArmed)
            }
        }
    }

    /// True if `f` is intact for `subject`: a wired static fuse, or a dynamic one armed with
    /// `subject` (as [`Registry::guard`]), that has not popped. Anything else reads false, so
    /// a reader takes its slow path. Not a GC point.
    pub(crate) fn intact(&self, rt: &Runtime, f: FuseRef, subject: u64) -> bool {
        self.guard(rt, f, subject).is_ok_and(|w| w.intact())
    }

    /// Pops `f`: a static fuse's word reads POPPED from now on and its live dependents are
    /// Invalidated; a dynamic fuse pops every armed subject ([`Registry::pop_subject`]). A
    /// popped fuse, or a dynamic one that is not armed, changes nothing. Not a GC point.
    pub(crate) fn pop(&self, rt: &Runtime, f: FuseRef) {
        match f {
            FuseRef::Static(i) => {
                let mut inner = self.inner.borrow_mut();
                if let Some(s) = inner.statics.get_mut(i as usize) {
                    s.word.0.set(POPPED);
                    s.dependents.invalidate_all();
                }
            }
            FuseRef::Dynamic(d) => {
                // One live subject at a time: each pop removes it (no list of subject values
                // is kept).
                while let Some(id) = self.first_subject(rt, d, |_, _| true) {
                    if !self.pop_subject(rt, d, id) {
                        break;
                    }
                }
            }
        }
    }

    /// Pops the armed subject `id` of `d` (one-way): its word reads POPPED; kinds 1-2 mark
    /// the dependents of every subject of `d` Invalidated (they are per isolate) and put `d`
    /// in the "popped in any realm" summary; kind 3 marks the subject's own dependents. The
    /// subject's handles are released; a weak one stays in the popped list. A subject that
    /// is not armed changes nothing. True if it popped one. Not a GC point.
    pub(crate) fn pop_subject(&self, rt: &Runtime, d: Descriptor, id: u64) -> bool {
        let mut inner = self.inner.borrow_mut();
        let Some(subjects) = inner.armed.get_mut(&d) else {
            return false;
        };
        let Some(at) = subjects.iter().position(|s| s.is(rt, id)) else {
            return false;
        };
        let mut s = subjects.swap_remove(at);
        s.word.0.set(POPPED);
        s.dependents.invalidate_all();
        if d.realm_wide() {
            for other in subjects.iter_mut() {
                other.dependents.invalidate_all();
            }
        }
        if subjects.is_empty() {
            inner.armed.remove(&d);
        }
        let n = self.counter(d);
        n.set(n.get() - 1);
        let object = s.release(rt);
        inner.popped.push(Popped { d, object });
        if d.realm_wide() {
            inner.popped_any.insert(d);
        }
        true
    }

    /// Drops dead dependents, dead subjects and dead popped entries, and the armed subjects
    /// that nothing uses (no live dependent and no word held outside the registry). Not a GC
    /// point.
    pub(crate) fn sweep(&self, rt: &Runtime) {
        let mut inner = self.inner.borrow_mut();
        for s in &mut inner.statics {
            s.dependents.sweep();
        }
        let mut empty = Vec::new();
        for (d, subjects) in &mut inner.armed {
            drop_dead_subjects(rt, subjects, &self.armed, &self.fields_armed, *d);
            let mut j = 0;
            while j < subjects.len() {
                let s = &mut subjects[j];
                s.dependents.sweep();
                if s.dependents.0.is_empty() && Rc::strong_count(&s.word) == 1 {
                    let s = subjects.swap_remove(j);
                    let n = self.counter(*d);
                    n.set(n.get() - 1);
                    if let Some(o) = s.release(rt) {
                        rt.heap.drop_weak(o);
                    }
                } else {
                    j += 1;
                }
            }
            if subjects.is_empty() {
                empty.push(*d);
            }
        }
        for d in empty {
            inner.armed.remove(&d);
        }
        inner.popped.retain(|p| match p.object {
            Some(o) if rt.heap.weak(o).is_none() => {
                rt.heap.drop_weak(o);
                false
            }
            _ => true,
        });
    }

    /// [`Registry::sweep`] if a GC ran since the last call: the runtime sweep point
    /// (`Cx::cleanup_finalization_registries`; module doc). Runs no JS. Not a GC point.
    pub(crate) fn sweep_after_gc(&self, rt: &Runtime) {
        if self.gc_since_sweep.replace(false) {
            self.sweep(rt);
        }
    }

    /// The entries of the dependents lists of `f` (every subject; dead ones not yet dropped
    /// included), or 0.
    pub(crate) fn dependent_entries(&self, f: FuseRef) -> usize {
        let inner = self.inner.borrow();
        match f {
            FuseRef::Static(i) => inner
                .statics
                .get(i as usize)
                .map_or(0, |s| s.dependents.0.len()),
            FuseRef::Dynamic(d) => inner
                .armed
                .get(&d)
                .map_or(0, |v| v.iter().map(|s| s.dependents.0.len()).sum()),
        }
    }

    /// The subjects a dynamic fuse is armed with (dead ones not yet dropped included), or 0.
    pub(crate) fn subject_count(&self, d: Descriptor) -> usize {
        self.inner.borrow().armed.get(&d).map_or(0, Vec::len)
    }

    /// The first live armed subject of `d` for which `pick` holds. Not a GC point.
    fn first_subject(
        &self,
        rt: &Runtime,
        d: Descriptor,
        pick: impl Fn(&Subject, u64) -> bool,
    ) -> Option<u64> {
        let inner = self.inner.borrow();
        inner
            .armed
            .get(&d)?
            .iter()
            .find_map(|s| s.id(rt).filter(|&id| pick(s, id)))
    }

    /// The first live armed kind-1/2 subject whose kept cell is no longer valid:
    /// (descriptor, subject). Not a GC point.
    #[cold]
    #[inline(never)]
    fn first_changed(&self, rt: &Runtime) -> Option<(Descriptor, u64)> {
        let inner = self.inner.borrow();
        inner.armed.iter().find_map(|(d, subjects)| {
            subjects.iter().find_map(|s| {
                let Watch::Cell(c) = s.watch else {
                    return None;
                };
                if validity::is_valid(rt.heap.global(c).to_bits()) {
                    return None;
                }
                s.id(rt).map(|id| (*d, id))
            })
        })
    }
}

/// The subject a fuse of `d` keeps for `subject` (`names` holds): kind 3 the field owner of
/// the shape, kinds 1-2 the subject itself. Not a GC point.
fn subject_id(d: Descriptor, subject: u64) -> u64 {
    match d {
        Descriptor::Field { index } => shape::field_owner(subject, usize::from(index)),
        _ => subject,
    }
}

/// The subject a read of `d` looks up for `subject` (`depend`, `guard`): kind 3 the field
/// owner of a shape the descriptor names (`None` for anything else); kinds 1-2 the subject
/// itself (the entry's identity and its cell decide, whatever the current realm). Not a GC
/// point.
fn lookup_id(rt: &Runtime, d: Descriptor, subject: u64) -> Option<u64> {
    if d.realm_wide() {
        return Some(subject);
    }
    names(rt, &d, subject).then(|| subject_id(d, subject))
}

/// Drops the subjects whose object died (a GC cleared the weak handle), with their handles.
fn drop_dead_subjects(
    rt: &Runtime,
    subjects: &mut Vec<Subject>,
    armed: &Cell<usize>,
    fields_armed: &Cell<usize>,
    d: Descriptor,
) {
    let mut j = 0;
    while j < subjects.len() {
        if subjects[j].object.is_none_or(|o| rt.heap.weak(o).is_some()) {
            j += 1;
            continue;
        }
        let s = subjects.swap_remove(j);
        let n = if d.realm_wide() { armed } else { fields_armed };
        n.set(n.get() - 1);
        if let Some(o) = s.release(rt) {
            rt.heap.drop_weak(o);
        }
    }
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
        Descriptor::Field { index } => {
            let i = usize::from(index);
            is_cell_bits(s)
                && shape::is_shape(s)
                && !shape::is_dictionary(s)
                && i < shape::count(s)
                && !shape::details_at(s, i).is_accessor()
        }
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
/// the validity cells are invalidated): pops every armed kind-1/2 subject whose cell is no
/// longer valid. One load and one branch while none is armed. Not a GC point.
#[inline(always)]
pub(crate) fn on_proto_change(rt: &Runtime, _o: u64) {
    if rt.fuses.armed.get() != 0 {
        pop_changed(rt);
    }
}

#[cold]
#[inline(never)]
fn pop_changed(rt: &Runtime) {
    // One subject at a time: each pop removes it.
    while let Some((d, id)) = rt.fuses.first_changed(rt) {
        if !rt.fuses.pop_subject(rt, d, id) {
            break;
        }
    }
}

/// The hook of a widening of descriptor `i` of some field family (`field_rep.rs`, after the
/// DESCRIPTORS cells are written): pops every armed kind-3 subject of index `i` whose field
/// bits changed. One load and one branch while none is armed. Not a GC point.
#[inline]
pub(crate) fn on_field_change(rt: &Runtime, i: usize) {
    if rt.fuses.fields_armed.get() != 0 {
        pop_fields_changed(rt, i);
    }
}

#[cold]
#[inline(never)]
fn pop_fields_changed(rt: &Runtime, i: usize) {
    let Ok(index) = u16::try_from(i) else {
        return;
    };
    let d = Descriptor::Field { index };
    // One subject at a time: each pop removes it.
    while let Some(id) = rt
        .fuses
        .first_subject(rt, d, |s, id| !s.current(rt, id, Some(i)))
    {
        if !rt.fuses.pop_subject(rt, d, id) {
            break;
        }
    }
}
