//! `Local<'s, T>`: a Value a builtin holds on the native stack for the scope `'s` (H20 ABI-2).
//!
//! A `Local` is the value itself (`repr(transparent)` over `Value`), not a handle. The
//! conservative stack scan (H10) keeps every cell that a Rust local holds alive, so a `Local`
//! costs nothing: no handle scope, no rooting call.
//!
//! | Property | How |
//! |---|---|
//! | 8 bytes, `JsResult<Local>` 8 bytes | `repr(transparent)`; `Value` wraps `NonZeroU64`, so `Err` uses the niche |
//! | Lives no longer than the host call | the brand `'s`; `HostFn` is `for<'s>` |
//! | Never in a static or on another thread | `'s` is not `'static`; `*const T` makes it `!Send + !Sync` |
//! | Never in Rust heap memory (`Vec`, `Box`, ...) | rule R2: `cargo xtask gc-lint` (P2) and the debug check of `&self` |
//!
//! The type parameter is a marker of what the value is: [`JsValue`] (any value, the default),
//! [`JsObject`], [`JsFunction`] (callable), [`JsArray`], [`JsString`], [`FlatStr`] (a flat
//! string, from `Cx::flatten`), [`JsSymbol`] and [`JsKey`] (a property key: an atom string or
//! a symbol, from the `Cx`). Downcasts are checked; upcasts are `From` conversions.
//!
//! Every accessor takes `&self` and reads the bits through [`Local::bits`], so a debug check
//! that `self` lies on the native stack (H20 lint plan, ABI-5 c) has one place to go.

use core::fmt;
use core::marker::PhantomData;

use ep9_value::Value;

use crate::kinds;
use crate::runtime::val;

/// Any value (the default marker of [`Local`]).
pub enum JsValue {}
/// An object (kinds 16-63).
pub enum JsObject {}
/// A callable object (IsCallable: a function or a bound function).
pub enum JsFunction {}
/// An Array exotic object.
pub enum JsArray {}
/// A string.
pub enum JsString {}
/// A flat string: its code units can be read (`Cx::flatten` makes one).
pub enum FlatStr {}
/// A symbol.
pub enum JsSymbol {}
/// A property key: an interned string (atom) or a symbol. Only the `Cx` makes one.
pub enum JsKey {}

mod sealed {
    pub trait Sealed {}
}

/// A marker that a checked downcast can test (every marker except [`JsKey`]).
pub trait Downcast: sealed::Sealed {
    /// The test on the value's bits.
    #[doc(hidden)]
    fn test(bits: u64) -> bool;
}

macro_rules! downcast {
    ($($m:ident => $test:expr,)*) => {$(
        impl sealed::Sealed for $m {}
        impl Downcast for $m {
            #[inline(always)]
            fn test(bits: u64) -> bool {
                let f: fn(u64) -> bool = $test;
                f(bits)
            }
        }
    )*};
}

downcast! {
    JsValue => |_| true,
    JsObject => kinds::is_object,
    JsFunction => kinds::is_callable,
    JsArray => kinds::is_array,
    JsString => kinds::is_string,
    // v0: every string is flat. Ropes (M1) make this a test of the flat kinds.
    FlatStr => kinds::is_string,
    JsSymbol => kinds::is_symbol,
}

/// A value held by Rust code for the host call `'s` (see the module docs).
#[repr(transparent)]
pub struct Local<'s, T = JsValue> {
    value: Value,
    _scope: PhantomData<(&'s (), *const T)>,
}

impl<T> Clone for Local<'_, T> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Local<'_, T> {}

impl<T> fmt::Debug for Local<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Local({:?})", self.value)
    }
}

impl<'s, T> Local<'s, T> {
    /// A `Local` of the bits `b`. The VM vouches that `b` holds a valid Value of kind `T`
    /// that is live (it sits in a frame, a root or a Rust local).
    #[inline(always)]
    pub(crate) fn from_bits(b: u64) -> Local<'s, T> {
        Local {
            value: val(b),
            _scope: PhantomData,
        }
    }

    /// The Value bits (the one read of a `Local`; see the module docs).
    #[inline(always)]
    pub(crate) fn bits(&self) -> u64 {
        self.value.to_bits()
    }

    /// The same value as a [`Local<JsValue>`](JsValue).
    #[inline(always)]
    pub fn value(&self) -> Local<'s> {
        Local::from_bits(self.bits())
    }

    /// The value as a `Local<U>` if it is one (a checked downcast).
    #[inline]
    pub fn downcast<U: Downcast>(&self) -> Option<Local<'s, U>> {
        let b = self.bits();
        U::test(b).then(|| Local::from_bits(b))
    }

    pub fn is_undefined(&self) -> bool {
        self.value.is_undefined()
    }

    pub fn is_null(&self) -> bool {
        self.value.is_null()
    }

    pub fn is_undefined_or_null(&self) -> bool {
        self.value.is_undefined_or_null()
    }

    pub fn is_number(&self) -> bool {
        self.value.is_number()
    }

    pub fn is_bool(&self) -> bool {
        self.value.is_bool()
    }

    pub fn is_string(&self) -> bool {
        kinds::is_string(self.bits())
    }

    pub fn is_symbol(&self) -> bool {
        kinds::is_symbol(self.bits())
    }

    pub fn is_object(&self) -> bool {
        kinds::is_object(self.bits())
    }

    /// IsCallable (§7.2.3).
    pub fn is_callable(&self) -> bool {
        kinds::is_callable(self.bits())
    }

    pub fn as_number(&self) -> Option<f64> {
        self.value.as_number()
    }

    pub fn as_int32(&self) -> Option<i32> {
        self.value.as_int32()
    }

    pub fn as_bool(&self) -> Option<bool> {
        self.value.as_bool()
    }

    pub fn as_object(&self) -> Option<Local<'s, JsObject>> {
        self.downcast()
    }

    pub fn as_function(&self) -> Option<Local<'s, JsFunction>> {
        self.downcast()
    }

    pub fn as_array(&self) -> Option<Local<'s, JsArray>> {
        self.downcast()
    }

    pub fn as_string(&self) -> Option<Local<'s, JsString>> {
        self.downcast()
    }

    pub fn as_symbol(&self) -> Option<Local<'s, JsSymbol>> {
        self.downcast()
    }

    /// True if both are the same value bits: the same cell, or the same immediate. This is
    /// not SameValue (two strings with equal text are different cells); see `Cx::same_value`.
    pub fn is_same_bits<U>(&self, other: &Local<'_, U>) -> bool {
        self.bits() == other.bits()
    }
}

impl<'s> Local<'s> {
    pub fn undefined() -> Local<'s> {
        Local::from_bits(Value::UNDEFINED.to_bits())
    }

    pub fn null() -> Local<'s> {
        Local::from_bits(Value::NULL.to_bits())
    }

    pub fn boolean(b: bool) -> Local<'s> {
        Local::from_bits(Value::bool(b).to_bits())
    }

    /// A number (int32 form when it fits, H6).
    pub fn number(d: f64) -> Local<'s> {
        Local::from_bits(Value::number(d).to_bits())
    }

    pub fn int32(i: i32) -> Local<'s> {
        Local::from_bits(Value::int32(i).to_bits())
    }
}

/// Upcasts: a narrower marker converts to a wider one without a check.
macro_rules! upcast {
    ($($from:ident => $to:ident,)*) => {$(
        impl<'s> From<Local<'s, $from>> for Local<'s, $to> {
            #[inline(always)]
            fn from(l: Local<'s, $from>) -> Self {
                Local::from_bits(l.bits())
            }
        }
    )*};
}

upcast! {
    JsObject => JsValue,
    JsFunction => JsValue,
    JsArray => JsValue,
    JsString => JsValue,
    FlatStr => JsValue,
    JsSymbol => JsValue,
    JsKey => JsValue,
    JsFunction => JsObject,
    JsArray => JsObject,
    FlatStr => JsString,
    JsSymbol => JsKey,
}
