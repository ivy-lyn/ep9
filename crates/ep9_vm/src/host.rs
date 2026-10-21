//! Host (Rust) functions callable from JS, and the context they get (H20 shape:
//! `fn(cx, this, args, new_target) -> JsResult<Value>`; the frozen builtin ABI is week-4 work).
//!
//! - `Args` is a view of the host frame's argument slots on the VM stack, so the arguments
//!   stay rooted while the host function runs.
//! - A host function may call back into JS ([`HostCx::call`]); that pushes a new entry frame
//!   above the host frame. The native stack is checked at every such entry.
//! - Values in Rust locals stay live (conservative stack scan, H10). Do not keep a `Value` in a
//!   `Vec` or other heap memory across a call that can allocate.

use core::marker::PhantomData;

use ep9_heap::Heap;
use ep9_value::{JsResult, Throw, Value};

use crate::conv;
use crate::error::throw_error;
use crate::interp::Cx;
use crate::object;
use crate::runtime::{ErrorKind, val};

/// A host function (H20 shape).
pub type HostFn = fn(&mut HostCx<'_>, Value, Args<'_>, Value) -> JsResult<Value>;

/// The arguments of a host call: a view of the host frame on the VM stack. Index `i >= len()`
/// reads `undefined`.
#[derive(Clone, Copy)]
pub struct Args<'a> {
    a0: *const u64,
    argc: u32,
    _frame: PhantomData<&'a [Value]>,
}

impl<'a> Args<'a> {
    /// # Safety
    /// `a0` is argument slot 0 of a live host frame with `argc` arguments stored downward
    /// (args reversed, frame.rs), and the frame outlives `'a`.
    pub(crate) unsafe fn new(a0: *const u64, argc: u32) -> Args<'a> {
        Args {
            a0,
            argc,
            _frame: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.argc as usize
    }

    pub fn is_empty(&self) -> bool {
        self.argc == 0
    }

    pub fn get(&self, i: usize) -> Value {
        if i < self.argc as usize {
            // SAFETY: slot i of the host frame (constructor contract); it holds a Value.
            val(unsafe { *self.a0.sub(i) })
        } else {
            Value::UNDEFINED
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = Value> + '_ {
        (0..self.len()).map(|i| self.get(i))
    }
}

/// What a host function can do with the isolate.
pub struct HostCx<'a> {
    pub(crate) cx: &'a mut Cx,
}

fn as_bits(args: &[Value]) -> &[u64] {
    // SAFETY: `Value` is `repr(transparent)` over `NonZeroU64`, which has the layout of `u64`.
    unsafe { core::slice::from_raw_parts(args.as_ptr().cast::<u64>(), args.len()) }
}

impl HostCx<'_> {
    /// Creates an error of `kind` and makes it the pending exception.
    pub fn throw_error(&mut self, kind: ErrorKind, message: &str) -> Throw {
        throw_error(self.cx, kind, message)
    }

    /// Makes `v` the pending exception.
    pub fn throw(&mut self, v: Value) -> Throw {
        self.cx.rt().throw_value(v.to_bits());
        Throw
    }

    pub fn new_string(&mut self, s: &str) -> Value {
        val(object::new_string(self.cx.rt(), s))
    }

    /// ToString, as Rust text.
    pub fn to_rust_string(&mut self, v: Value) -> JsResult<String> {
        conv::to_rust_string(self.cx, v.to_bits())
    }

    pub fn to_number(&mut self, v: Value) -> JsResult<f64> {
        conv::to_number(self.cx, v.to_bits())
    }

    /// Call(f, this, args): runs JS on a new entry frame.
    pub fn call(&mut self, f: Value, this: Value, args: &[Value]) -> JsResult<Value> {
        crate::call::call_function(self.cx, f.to_bits(), this.to_bits(), as_bits(args)).map(val)
    }

    /// Construct(f, args).
    pub fn construct(&mut self, f: Value, args: &[Value]) -> JsResult<Value> {
        crate::call::construct(self.cx, f.to_bits(), as_bits(args)).map(val)
    }

    /// `obj[key]`.
    pub fn get(&mut self, obj: Value, key: &str) -> JsResult<Value> {
        let k = crate::atoms::intern_str(self.cx.rt(), key);
        crate::property::get(self.cx, obj.to_bits(), k).map(val)
    }

    /// `obj[key] = v` (strict).
    pub fn set(&mut self, obj: Value, key: &str, v: Value) -> JsResult<()> {
        let k = crate::atoms::intern_str(self.cx.rt(), key);
        crate::property::set(self.cx, obj.to_bits(), k, v.to_bits(), true)
    }

    pub fn global(&self) -> Value {
        val(self.cx.rt().global())
    }

    pub fn new_object(&mut self) -> Value {
        val(object::new_plain_object(self.cx.rt()))
    }

    pub fn is_callable(&self, v: Value) -> bool {
        crate::kinds::is_callable(v.to_bits())
    }

    /// `typeof v`.
    pub fn type_of(&self, v: Value) -> &'static str {
        conv::type_of(v.to_bits()).text()
    }

    /// Runs a full GC now (tests, `gc()` hooks).
    pub fn collect_garbage(&mut self) {
        self.cx.rt().heap.collect();
    }

    pub fn heap(&self) -> &Heap {
        &self.cx.rt().heap
    }
}
