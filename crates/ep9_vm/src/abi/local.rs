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
//! | Never in Rust heap memory (`Vec`, `Box`, ...) | rule R2: `cargo xtask gc-lint`, and in debug builds every read checks that `self` lies on the thread's native stack |
//!
//! The type parameter is a marker of what the value is: [`JsValue`] (any value, the default),
//! [`JsObject`], [`JsFunction`] (callable), [`JsArray`], [`JsString`], [`FlatStr`] (a flat
//! string, from `Cx::flatten`), [`JsSymbol`] and [`JsKey`] (a property key: an atom string or
//! a symbol, from the `Cx`). Downcasts are checked; upcasts are `From` conversions.
//!
//! Every accessor takes `&self` and reads the value through one function, so the debug check
//! that `self` lies on the native stack (H20 ABI-5 c, as V8's `V8_ENABLE_LOCAL_OFF_STACK_CHECK`)
//! has one place to go. A `Local` read through `&vec[0]` or a `Box` panics in a debug build.
//! The check misses a `Local` copied out of heap memory before the read (`let l = v[0];`);
//! `cargo xtask gc-lint` and the GC-stress runs cover that (H10 risk R3).

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
    /// Only this crate can make a `Token` (its field is private).
    pub struct Token(());

    impl Token {
        #[inline(always)]
        pub(crate) fn new() -> Token {
            Token(())
        }
    }

    pub trait Sealed {
        /// The test on the value's bits. The kind tests read the cell header, so only the VM
        /// calls it, on the bits of a live value. Code outside the crate can reach a supertrait
        /// method through a `T: Downcast` bound, but it cannot make the `Token` (RV8-1).
        fn test(bits: u64, _: Token) -> bool;
    }
}

/// A marker that a checked downcast can test (every marker except [`JsKey`]).
///
/// The test on the bits is not part of this trait. Code outside the crate cannot call it on
/// bits that it chose:
///
/// ```compile_fail
/// use ep9_vm::{Downcast, JsString};
/// let _ = <JsString as Downcast>::test(0x80_0000);
/// ```
///
/// A `T: Downcast` bound reaches `test` (a supertrait method), but the call needs a
/// `sealed::Token`, and code outside the crate cannot make one: the type has a private field,
/// no constructor outside the crate and no `Default`. A diverging argument (`todo!()`) type
/// checks, but it stops before the call.
pub trait Downcast: sealed::Sealed {}

macro_rules! downcast {
    ($($m:ident => $test:expr,)*) => {$(
        impl sealed::Sealed for $m {
            #[inline(always)]
            fn test(bits: u64, _: sealed::Token) -> bool {
                let f: fn(u64) -> bool = $test;
                f(bits)
            }
        }
        impl Downcast for $m {}
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
///
/// An upcast keeps the scope: each `From` impl of the upcast list converts `Local<'s, A>` to
/// `Local<'s, B>` with the same `'s`, so `.into()` cannot take a `Local` out of
/// `Isolate::with_cx` (the H20 core golden holds these impl signatures). The same upcast
/// inside the scope compiles:
///
/// ```
/// use ep9_vm::{Isolate, IsolateConfig, JsObject, JsValue, Local};
/// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
/// let is_object = iso.with_cx(|cx| {
///     let v: Local<'_, JsValue> = cx.global().into();
///     Ok(v.downcast::<JsObject>().is_some())
/// });
/// assert_eq!(is_object, Ok(true));
/// ```
///
/// An escape through `.into()` does not compile:
///
/// ```compile_fail
/// use ep9_vm::{Isolate, IsolateConfig, JsValue, Local};
/// let mut iso = Isolate::new(IsolateConfig::default()).unwrap();
/// let global: Local<'static, JsValue> = iso.with_cx(|cx| Ok(cx.global().into())).unwrap();
/// iso.collect_garbage();
/// let _ = global;
/// ```
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
        // Not checked: it prints the bits and reads no cell, also in a panic message.
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

    /// The value: the one read of a `Local` (see the module docs). Debug builds check that
    /// `self` lies on this thread's native stack.
    #[inline(always)]
    fn get(&self) -> Value {
        #[cfg(all(debug_assertions, not(miri)))]
        on_stack::check(core::ptr::from_ref(self).addr());
        self.value
    }

    /// The Value bits.
    #[inline(always)]
    pub(crate) fn bits(&self) -> u64 {
        self.get().to_bits()
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
        <U as sealed::Sealed>::test(b, sealed::Token::new()).then(|| Local::from_bits(b))
    }

    pub fn is_undefined(&self) -> bool {
        self.get().is_undefined()
    }

    pub fn is_null(&self) -> bool {
        self.get().is_null()
    }

    pub fn is_undefined_or_null(&self) -> bool {
        self.get().is_undefined_or_null()
    }

    pub fn is_number(&self) -> bool {
        self.get().is_number()
    }

    pub fn is_bool(&self) -> bool {
        self.get().is_bool()
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
        self.get().as_number()
    }

    pub fn as_int32(&self) -> Option<i32> {
        self.get().as_int32()
    }

    pub fn as_bool(&self) -> Option<bool> {
        self.get().as_bool()
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

/// The debug on-stack check (H20 ABI-5 c). The bounds come from the OS once per thread and are
/// cached in a `thread_local` \[I48\]: on the Linux main thread the OS call reads
/// `/proc/self/maps`.
#[cfg(all(debug_assertions, not(miri)))]
mod on_stack {
    use core::cell::Cell;

    thread_local! {
        /// `[low, high)` of this thread's stack: `(0, 0)` before the first read, `(0, MAX)`
        /// if the OS gave no bounds (then nothing is checked).
        static BOUNDS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
    }

    /// Panics unless `addr` (a `&Local`) lies on the calling thread's stack.
    #[inline(never)]
    pub(super) fn check(addr: usize) {
        let (low, high) = BOUNDS.with(|b| {
            if b.get().1 == 0 {
                b.set(match ep9_platform::current_thread_stack_bounds() {
                    Ok(s) => (s.low, s.high),
                    Err(_) => (0, usize::MAX),
                });
            }
            b.get()
        });
        assert!(
            low <= addr && addr < high,
            "a Local was read outside the native stack ({addr:#x} is not in \
             [{low:#x}, {high:#x})): a Local lives in a Rust local, never in Rust heap memory \
             (H20 rule R2; `cargo xtask gc-lint` finds the container)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locals_on_the_stack_read() {
        let l = Local::int32(7);
        assert_eq!(l.as_int32(), Some(7));
        let pair = [Local::undefined(), Local::null()];
        assert!(pair[0].is_undefined() && pair[1].is_null());
        assert!(pair[0].value().is_same_bits(&Local::undefined()));
        // Another thread reads its own bounds.
        std::thread::spawn(|| {
            let b = Local::boolean(true);
            assert_eq!(b.as_bool(), Some(true));
        })
        .join()
        .unwrap();
    }

    // gc-lint: allow(debug test: a Local in a Vec must make the on-stack check panic)
    #[test]
    #[cfg(all(debug_assertions, not(miri)))]
    #[should_panic(expected = "a Local was read outside the native stack")]
    fn a_local_read_through_a_vec_panics() {
        let v: Vec<Local<'_>> = vec![Local::int32(1)];
        let first = &v[0];
        let _ = first.as_int32();
    }
}
