//! Index access on the builtin ABI (H20 ABI-1): `get_index`, `has_index`, `set_index` and
//! `delete_index`, the ObjectOps of the key `! ToString(𝔽(index))` for an integer index.
//!
//! ```text
//! cx.get_index(o, k)  ──> property::get_index: a dense element, a String wrapper index or a
//!                         missing index answers without a key (no atom); a stored integer key
//!                         is found by a scan of a small object; else the key path (an atom)
//! cx.set_index(o, k, v, throw) ──> a dense array element in place, else Set(O, key, V, throw)
//! ```
//!
//! An index is a `u64`, as `length_of_array_like` gives it (at most 2^53 - 1 from ToLength);
//! a larger one is converted as the spec does, through `𝔽(index)`.
//!
//! A loop over indices has no back edge, and a missing index runs no JS, so nothing polls in
//! it: a walk to 2^53 - 1 calls [`Cx::poll_interrupt`] once per index.

use ep9_value::{JsResult, Throw};

use super::{Cx, JsObject, Local};
use crate::{interp, property};

impl<'s> Cx<'s> {
    /// Get(O, ! ToString(𝔽(index))) (§7.3.2). Makes no atom for a dense element or a missing
    /// index.
    pub fn get_index(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        index: u64,
    ) -> JsResult<Local<'s>> {
        let o = o.into().bits();
        property::get_index(self.cx, o, index).map(Self::local)
    }

    /// HasProperty(O, ! ToString(𝔽(index))) (§7.3.12). Makes no atom for a dense element or
    /// a missing index. Any object: a proxy, or a proxy on the chain, answers through its
    /// `[[HasProperty]]` (a revoked one throws a TypeError; M4-P4 fixer B, Codex PROXY1 P-3).
    pub fn has_index(&mut self, o: impl Into<Local<'s, JsObject>>, index: u64) -> JsResult<bool> {
        let o = o.into().bits();
        property::try_has_index(self.cx, o, index)
    }

    /// Set(O, ! ToString(𝔽(index)), V, Throw) (§7.3.4). `Ok(false)` if the assignment failed
    /// and `throw` is false; a TypeError if it failed and `throw` is true. A dense array
    /// element is stored without a key.
    pub fn set_index(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        index: u64,
        v: impl Into<Local<'s>>,
        throw: bool,
    ) -> JsResult<bool> {
        let (o, v) = (o.into().bits(), v.into().bits());
        if property::set_index(self.cx, o, index, v)? {
            return Ok(true);
        }
        if throw {
            let msg = format!("Cannot assign to read only property '{index}' of object");
            return Err(self.throw_type_error(&msg));
        }
        Ok(false)
    }

    /// CreateDataPropertyOrThrow(O, ! ToString(𝔽(index)), V) (§7.3.7). A dense array element
    /// is stored without a key (no atom).
    pub fn create_data_property_index_or_throw(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        index: u64,
        v: impl Into<Local<'s>>,
    ) -> JsResult<()> {
        let (o, v) = (o.into().bits(), v.into().bits());
        property::define_index(self.cx, o, index, v)
    }

    /// O.`[[Delete]]`(! ToString(𝔽(index))) (§10.1.10) with `throw` false, or
    /// DeletePropertyOrThrow (§7.3.10) with `throw` true (a TypeError instead of `false`). An
    /// element becomes a hole, and a missing index gives true, without a key.
    pub fn delete_index(
        &mut self,
        o: impl Into<Local<'s, JsObject>>,
        index: u64,
        throw: bool,
    ) -> JsResult<bool> {
        let o = o.into().bits();
        property::delete_index(self.cx, o, index, throw)
    }

    /// Takes one interrupt budget unit and polls when the budget runs out, as a loop back
    /// edge does: a builtin loop with no back edge and no JS call (a walk over holes) calls
    /// it once per pass. `Err(Throw)` with the termination pending (it cannot be caught) if
    /// the isolate must stop. Allocates nothing.
    #[inline]
    pub fn poll_interrupt(&mut self) -> JsResult<()> {
        self.cx.budget -= 1;
        if self.cx.budget <= 0 && interp::slow::interrupt(self.cx, core::ptr::null_mut()) == 0 {
            return Err(Throw);
        }
        Ok(())
    }

    /// The native stack check of a builtin that recurses in Rust (H23 G6): `Err` with the
    /// catchable RangeError "Maximum call stack size exceeded" (the VM's own stack overflow
    /// error) once the native stack passes the isolate's limit (`Runtime::native_stack_ok`),
    /// else `Ok`. A recursive builtin calls it at the top of each recursion step, so a deep
    /// input throws instead of overflowing the native stack.
    #[inline]
    pub fn check_native_stack(&mut self) -> JsResult<()> {
        if self.rt().native_stack_ok() {
            return Ok(());
        }
        Err(self.throw_range_error("Maximum call stack size exceeded"))
    }
}
