//! Realms on the builtin ABI (H22-Q3, H20 R5): making a realm and running code in it. A realm
//! is named by its global object. The realm machinery is `realm.rs`; the realm of a function
//! is its `[[Realm]]` (`object::function_realm`, OBJECT_LAYOUT 3), and a call of a function of
//! another realm switches to it (`realm::switch`), so a host function always runs in its own
//! realm: what it makes through this `Cx` (objects, errors, intrinsics) belongs to that realm.
//!
//! ```text
//! cx.create_realm()                     a new realm, all builtin tables; the current realm stays
//! cx.with_realm(global, |cx| ..)        runs Rust code with that realm current, then switches back
//! cx.evaluate_script_in(global, src, f) ScriptEvaluation in that realm (Cx::evaluate_script)
//! cx.function_realm_intrinsic(f, id)    an intrinsic of GetFunctionRealm(f); try_: the ? form
//! ```

use ep9_value::JsResult;

use super::{Cx, JsObject, Local};
use crate::builtins::IntrinsicId;
use crate::realm;
use crate::runtime::ErrorKind;

impl<'s> Cx<'s> {
    /// CreateRealm (§9.3.1) with the isolate's builtin tables (the VM table, then
    /// `IsolateConfig::builtins`): a new realm with its own intrinsics and global object.
    /// Returns that global object; the current realm does not change. The realm lives while
    /// something reachable holds one of its objects or functions. GC point.
    ///
    /// # Errors
    ///
    /// An `Error` if the install fails (not expected: the tables were checked and installed
    /// once already).
    pub fn create_realm(&mut self) -> JsResult<Local<'s, JsObject>> {
        let r = realm::create(self.cx)?;
        Ok(Self::local(object_of(r)))
    }

    /// Runs `f` with the realm whose global object is `global` as the current realm (as a
    /// call of one of its functions does), and switches back when `f` returns, also on `Err`.
    ///
    /// # Errors
    ///
    /// A `TypeError` if `global` is not the global object of a realm of this isolate;
    /// whatever `f` returns.
    pub fn with_realm<R>(
        &mut self,
        global: impl Into<Local<'s, JsObject>>,
        f: impl FnOnce(&mut Self) -> JsResult<R>,
    ) -> JsResult<R> {
        let g = global.into().bits();
        let Some(target) = realm::of_global(self.rt(), g) else {
            return Err(self.throw_error(
                ErrorKind::TypeError,
                "not the global object of a realm of this isolate",
            ));
        };
        let saved = self.cx.realm;
        if target == saved {
            return f(self);
        }
        realm::switch(self.cx, target);
        let r = f(self);
        realm::switch(self.cx, saved);
        r
    }

    /// [`Cx::evaluate_script`] in the realm whose global object is `global`: the script's
    /// global declarations, its `this` and every object it makes belong to that realm.
    ///
    /// # Errors
    ///
    /// As [`Cx::evaluate_script`]; a `TypeError` if `global` is not a realm's global object.
    pub fn evaluate_script_in(
        &mut self,
        global: impl Into<Local<'s, JsObject>>,
        source: &str,
        file: &str,
    ) -> JsResult<Local<'s>> {
        self.with_realm(global, |cx| cx.evaluate_script(source, file))
    }

    /// The intrinsic `id` of GetFunctionRealm(`f`) (§7.3.22: the realm of a function, of a
    /// bound function's target, else the current realm), or undefined if no table made it.
    /// It never throws: a revoked proxy on the walk gives the current realm; a spec step
    /// with `? GetFunctionRealm` uses [`Cx::try_function_realm_intrinsic`]. Not a GC point.
    pub fn function_realm_intrinsic(&self, f: impl Into<Local<'s>>, id: IntrinsicId) -> Local<'s> {
        let rt = self.rt();
        let r = realm::get_function_realm(rt, f.into().bits());
        Self::local(realm::intrinsic_in(rt, r, id))
    }

    /// [`Cx::function_realm_intrinsic`] with GetFunctionRealm step 4.a (§7.3.22): a revoked
    /// proxy on the walk (also behind a bound function) throws a TypeError. For the spec
    /// steps that take `? GetFunctionRealm(C)`, as ArraySpeciesCreate step 4.b (§10.4.2.3).
    /// Not a GC point before the throw.
    ///
    /// # Errors
    ///
    /// A `TypeError` if the walk reaches a revoked proxy.
    pub fn try_function_realm_intrinsic(
        &mut self,
        f: impl Into<Local<'s>>,
        id: IntrinsicId,
    ) -> JsResult<Local<'s>> {
        let r = realm::try_get_function_realm(self.cx, f.into().bits())?;
        Ok(Self::local(realm::intrinsic_in(self.rt(), r, id)))
    }
}

/// The global object of the REALM record `r`.
fn object_of(r: u64) -> u64 {
    crate::object::record_get(r, crate::object::realmf::GLOBAL_OBJECT)
}
