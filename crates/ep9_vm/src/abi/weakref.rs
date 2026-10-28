//! WeakRef and FinalizationRegistry on the builtin ABI (owner M4-P4 WEAKREF; GFF-WEAKREF,
//! V9-D23): the part-2 `Cx` methods that `ep9_builtins` weakref.rs uses, over `weakref.rs`
//! (the WEAK_CELL kind and `Runtime::kept_objects`), and the two job-end steps that the job
//! loop (or an embedder) runs.
//!
//! ```text
//! ep9_builtins weakref.rs ──new_weak_ref / weak_ref_deref──────────────> weakref.rs (VM)
//!   (the spec steps that throw)  new_finalization_registry / ..._register / ..._unregister
//! the job loop, queue empty:    clear_kept_objects            (ClearKeptObjects)
//!   then:                        cleanup_finalization_registries (the cleanup jobs)
//! ```
//!
//! | Method | Spec | Errors |
//! |---|---|---|
//! | `new_weak_ref(proto, target)` | WeakRef ( target ) (§26.1.1.1) steps 3-5 for a `proto` from GetPrototypeFromConstructor: the object, AddToKeptObjects(target), `[[WeakRefTarget]]` | none; panics if CanBeHeldWeakly(target) is false (the caller throws the step-2 TypeError) |
//! | `weak_ref_deref(w)` | WeakRef.prototype.deref (§26.1.3.2): RequireInternalSlot, then WeakRefDeref (§26.1.4.1: the target, kept alive to the end of the job, or undefined) | TypeError if `w` is not a WeakRef |
//! | `new_finalization_registry(proto, cleanup)` | FinalizationRegistry ( cleanupCallback ) (§26.2.1.1) steps 3-8 for a `proto` from GetPrototypeFromConstructor | none; panics if `cleanup` is not callable (the caller throws the step-2 TypeError) |
//! | `finalization_registry_register(r, target, held, token)` | FinalizationRegistry.prototype.register (§26.2.3.2) steps 2 and 6-7: RequireInternalSlot, then a new cell appended to `[[Cells]]`; `token` undefined = empty | TypeError if `r` is not a FinalizationRegistry; panics if `target` or a `token` that is not undefined fails CanBeHeldWeakly (the caller throws steps 3-5 first) |
//! | `finalization_registry_unregister(r, token)` | FinalizationRegistry.prototype.unregister (§26.2.3.3) steps 2 and 4-6: removes every cell whose token is `token`; true if one was removed | TypeError if `r` is not a FinalizationRegistry; panics if CanBeHeldWeakly(token) is false (the caller throws step 3 first) |
//! | `clear_kept_objects()` | ClearKeptObjects (§9.10.1): the job loop runs it when its queue is empty, so a target stays alive through every job of one checkpoint (HTML's microtask checkpoint and node v24.22.1 do the same) | none |
//! | `cleanup_finalization_registries()` | the cleanup jobs (HostEnqueueFinalizationRegistryCleanupJob §9.5.4, CleanupFinalizationRegistry §9.13.1): for each live registry, each cell whose target is empty is removed from `[[Cells]]` and the callback is called with its held value | the callback's throw (the cells not yet done stay for the next call); an Error with JS on the stack (it runs only between jobs, as a microtask checkpoint does) |
//!
//! A GC never runs a callback: its weak source only notes that a target or a registry died
//! (`weakref.rs`). The callbacks run in `cleanup_finalization_registries`, which the job loop
//! calls after the queue is empty (07-runtime.md: promise jobs, then ClearKeptObjects, then the
//! FinalizationRegistry cleanup). A callback may queue jobs, register, unregister or collect.
//!
//! Each method is a part-2 addition: an `h20-core.txt` line (EP9_BLESS) and an H20 Change-log
//! row (V9-A1, V9-A14).

use ep9_value::JsResult;

use super::{Cx, JsObject, Local, ObjectClass};
use crate::conv;
use crate::frame::UNDEFINED;
use crate::kinds;
use crate::object;
use crate::runtime::ErrorKind;
use crate::weakref::{self, registry, weak_ref};

impl<'s> Cx<'s> {
    /// The bits of `o` if it is a WeakRef made by [`Cx::new_weak_ref`] or a FinalizationRegistry
    /// made by [`Cx::new_finalization_registry`] (`class`), else a TypeError
    /// (RequireInternalSlot, §10.1.15). An object of the class with other slots (made with
    /// `new_object_with_slots`, or changed with `set_slot`) is not one.
    fn weak_object(&mut self, o: Local<'s>, class: ObjectClass) -> JsResult<u64> {
        let b = o.bits();
        if object::slot_class(b) == Some(class.id()) && weak_slots_ok(b, class) {
            return Ok(b);
        }
        let msg = format!("{} is not a {} object", conv::describe(b), class.name());
        Err(self.throw_type_error(&msg))
    }

    /// WeakRef ( target ) (§26.1.1.1) steps 3-5: a WeakRef object with the prototype `proto`
    /// (from [`Cx::get_prototype_from_constructor`]: OrdinaryCreateFromConstructor), then
    /// AddToKeptObjects(target), then `[[WeakRefTarget]]` = `target`, held weakly. Panics if
    /// `proto` is not an object or null, or if [`Cx::can_be_held_weakly`] is false for
    /// `target` (the caller throws the TypeError of step 2 first).
    pub fn new_weak_ref(
        &mut self,
        proto: impl Into<Local<'s>>,
        target: impl Into<Local<'s>>,
    ) -> Local<'s, JsObject> {
        let target = target.into();
        assert!(
            self.can_be_held_weakly(target),
            "a WeakRef target must be an object or a symbol that is not registered"
        );
        let o = self.new_object_with_slots(proto, ObjectClass::WeakRef, weak_ref::COUNT);
        let rt = self.rt();
        let t = target.bits();
        // GC points: `o` and `t` live in Rust locals (CSS).
        rt.kept_objects.add(rt, t);
        let c = weakref::new_cell(rt, t, UNDEFINED, UNDEFINED, UNDEFINED);
        object::slot_set(rt, o.bits(), weak_ref::CELL, c);
        o
    }

    /// WeakRef.prototype.deref ( ) (§26.1.3.2) for the this value `w`: RequireInternalSlot,
    /// then WeakRefDeref (§26.1.4.1): the target, which AddToKeptObjects keeps alive until
    /// the next [`Cx::clear_kept_objects`], or undefined once it died. A TypeError if `w` is
    /// not a WeakRef.
    pub fn weak_ref_deref(&mut self, w: impl Into<Local<'s>>) -> JsResult<Local<'s>> {
        let o = self.weak_object(w.into(), ObjectClass::WeakRef)?;
        let t = weakref::target(object::slot_get(o, weak_ref::CELL));
        if t == UNDEFINED {
            return Ok(Local::undefined());
        }
        let rt = self.rt();
        // GC point: `t` lives in a Rust local.
        rt.kept_objects.add(rt, t);
        Ok(Self::local(t))
    }

    /// FinalizationRegistry ( cleanupCallback ) (§26.2.1.1) steps 3-8: a FinalizationRegistry
    /// object with the prototype `proto` (from [`Cx::get_prototype_from_constructor`]), the
    /// callback `cleanup` and no cells. Panics if `proto` is not an object or null, or if
    /// `cleanup` is not callable (the caller throws the TypeError of step 2 first).
    pub fn new_finalization_registry(
        &mut self,
        proto: impl Into<Local<'s>>,
        cleanup: impl Into<Local<'s>>,
    ) -> Local<'s, JsObject> {
        let cleanup = cleanup.into();
        assert!(
            cleanup.is_callable(),
            "a FinalizationRegistry cleanup callback must be callable"
        );
        let o =
            self.new_object_with_slots(proto, ObjectClass::FinalizationRegistry, registry::COUNT);
        let rt = self.rt();
        let fr = o.bits();
        object::slot_set(rt, fr, registry::CLEANUP, cleanup.bits());
        // GC point: `fr` lives in a Rust local.
        rt.kept_objects.add_registry(rt, fr);
        o
    }

    /// FinalizationRegistry.prototype.register ( target, heldValue \[ , unregisterToken \] )
    /// (§26.2.3.2) steps 2 and 6-7 for the this value `r`: RequireInternalSlot, then a new
    /// cell { `target` (weak), `held`, `token` (weak; undefined = empty) } in `[[Cells]]`. A
    /// TypeError if `r` is not a FinalizationRegistry. Panics if [`Cx::can_be_held_weakly`] is
    /// false for `target`, or for a `token` that is not undefined (the caller throws the
    /// TypeErrors of steps 3-5 first; step 4, `target` SameValue `held`, is the caller's too).
    pub fn finalization_registry_register(
        &mut self,
        r: impl Into<Local<'s>>,
        target: impl Into<Local<'s>>,
        held: impl Into<Local<'s>>,
        token: impl Into<Local<'s>>,
    ) -> JsResult<()> {
        let fr = self.weak_object(r.into(), ObjectClass::FinalizationRegistry)?;
        let (target, held, token) = (target.into(), held.into(), token.into());
        assert!(
            self.can_be_held_weakly(target),
            "a FinalizationRegistry target must be an object or a symbol that is not registered"
        );
        assert!(
            token.is_undefined() || self.can_be_held_weakly(token),
            "an unregister token must be undefined, an object or a symbol that is not registered"
        );
        let rt = self.rt();
        let head = object::slot_get(fr, registry::CELLS);
        // GC point: the values live in Rust locals; a GC changes no list head.
        let c = weakref::new_cell(rt, target.bits(), held.bits(), token.bits(), head);
        object::slot_set(rt, fr, registry::CELLS, c);
        Ok(())
    }

    /// FinalizationRegistry.prototype.unregister ( unregisterToken ) (§26.2.3.3) steps 2 and
    /// 4-6 for the this value `r`: RequireInternalSlot, then every cell of `[[Cells]]` whose
    /// token is `token` is removed (also a cell whose target died and whose callback has not
    /// run); true if one was. A TypeError if `r` is not a FinalizationRegistry. Panics if
    /// [`Cx::can_be_held_weakly`] is false for `token` (the caller throws step 3 first).
    pub fn finalization_registry_unregister(
        &mut self,
        r: impl Into<Local<'s>>,
        token: impl Into<Local<'s>>,
    ) -> JsResult<bool> {
        let fr = self.weak_object(r.into(), ObjectClass::FinalizationRegistry)?;
        let token = token.into();
        assert!(
            self.can_be_held_weakly(token),
            "an unregister token must be an object or a symbol that is not registered"
        );
        let rt = self.rt();
        let n = weakref::remove_token(rt, fr, registry::CELLS, token.bits())
            + weakref::remove_token(rt, fr, registry::CLEARED, token.bits());
        Ok(n > 0)
    }

    /// ClearKeptObjects (§9.10.1): the targets that WeakRef creation and `deref` kept alive
    /// since the last call may die at the next GC. The job loop calls it each time its queue is
    /// empty (before the cleanup jobs) and at the end of a microtask checkpoint (so after each
    /// script, its jobs and the cleanup callbacks), as HTML and node do; an embedder that
    /// runs JS with no checkpoint calls it when its synchronous run ends. Never throws; not a
    /// GC point.
    pub fn clear_kept_objects(&mut self) {
        self.rt().kept_objects.clear();
    }

    /// The FinalizationRegistry cleanup jobs (HostEnqueueFinalizationRegistryCleanupJob
    /// §9.5.4, CleanupFinalizationRegistry §9.13.1): for each registry that is still alive,
    /// each cell whose target died since the last call is removed from `[[Cells]]`, and the
    /// registry's callback is called with `this` undefined and the cell's held value. O(1)
    /// when no GC since the last call saw a target or a registry die. Each callback takes one
    /// interrupt budget unit and polls before it runs, as a job does ([`Cx::poll_interrupt`];
    /// a builtin callback such as `Boolean` runs no poll of its own; M4-P4 P4-ORCH-8, Codex
    /// C2-1), so an interrupt callback runs and a termination stops the run; a list walk that
    /// stopped at a termination request (`weakref::walk_step`) runs no callback. A callback that
    /// throws, or a termination, stops the run with its exception (`Err`); the cells not yet
    /// done stay, and the next call does them. Only with no JS on the stack (as
    /// [`Cx::perform_microtask_checkpoint`]): from a host function it throws an Error and runs
    /// nothing. The job loop calls it after the job queue is empty and after
    /// [`Cx::clear_kept_objects`]; jobs that a callback queues run after it. It is also the
    /// fuse registry's sweep point: after a GC it first drops the fuses that nothing uses
    /// (`fuse::Registry::sweep_after_gc`; no JS, not a GC point).
    pub fn cleanup_finalization_registries(&mut self) -> JsResult<()> {
        let rt = self.rt();
        if !rt.stack().fp().is_null() {
            return Err(self.throw_error(
                ErrorKind::Error,
                "FinalizationRegistry cleanup needs an empty JS stack",
            ));
        }
        rt.fuses.sweep_after_gc(rt);
        if !rt.kept_objects.take_pending() {
            return Ok(());
        }
        weakref::unlink_dead_registries(rt);
        let mut link = rt.kept_objects.registries();
        while link != UNDEFINED {
            weakref::collect_cleared(rt, weakref::target(link));
            link = weakref::next(link);
        }
        // A walk that stopped at a termination request (`weakref::walk_step`): no callback
        // runs after it, so the first poll below ends the run with the termination.
        if rt.kept_objects.walk_stopped() {
            self.cx.budget = 0;
        }
        // The callbacks. Each one may run any JS and collect, so every read is fresh; `link`
        // and `fr` live in Rust locals (CSS), so the registry stays alive while it is served.
        let mut link = rt.kept_objects.registries();
        while link != UNDEFINED {
            let fr = weakref::target(link);
            if weakref::is_registry(fr) {
                loop {
                    let c = object::slot_get(fr, registry::CLEARED);
                    if !weakref::is_weak_cell(c) {
                        break;
                    }
                    // One budget unit per callback, and a poll when it runs out: the cell
                    // stays in CLEARED if the poll stops the run. The poll runs no JS and
                    // allocates nothing (H23-Q6).
                    if let Err(t) = self.poll_interrupt() {
                        rt.kept_objects.set_pending();
                        return Err(t);
                    }
                    // §9.13.1 step 3.b: remove the cell from [[Cells]], then c: call.
                    object::slot_set(rt, fr, registry::CLEARED, weakref::next(c));
                    let held: Local<'s> = Self::local(weakref::held(c));
                    let callback: Local<'s> = Self::local(object::slot_get(fr, registry::CLEANUP));
                    if let Err(t) = self.call(callback, Local::undefined(), &[held]) {
                        rt.kept_objects.set_pending();
                        return Err(t);
                    }
                }
            }
            link = weakref::next(link);
        }
        Ok(())
    }
}

/// True if the SLOT_OBJECT `o` of `class` has the slots that this file made: a WeakRef has
/// one slot holding a WEAK_CELL; a FinalizationRegistry has [`registry::COUNT`] slots and a
/// callable callback.
fn weak_slots_ok(o: u64, class: ObjectClass) -> bool {
    match class {
        ObjectClass::WeakRef => {
            object::slot_count(o) == weak_ref::COUNT
                && kinds::cell_kind(object::slot_get(o, weak_ref::CELL)) == kinds::WEAK_CELL.get()
        }
        _ => {
            object::slot_count(o) == registry::COUNT
                && kinds::is_callable(object::slot_get(o, registry::CLEANUP))
        }
    }
}
