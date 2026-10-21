//! The handler DSL (H15), ported from `spikes/sp-dispatch/src/dsl.rs`.
//!
//! Input: the entries of `ep9_bytecode::for_each_opcode!` (name, byte, operand kinds).
//! Bodies: `ops.rs`, one module per opcode (`ops::<Name>::fast` and maybe `ops::<Name>::cold`).
//! `ops::if_cold!` lists the opcodes with a cold block; it picks the tokens for each case, and
//! the body types (`Flow` with a cold block, `Done` without) make a wrong list a type error.
//!
//! | Macro | Output |
//! |---|---|
//! | `expand_threaded!([return, "sib"])` | (b) `ep9h__sib__tail__<Name>` per opcode, `..._cold` per cold block, `TABLE` |
//! | `expand_threaded!([become, "become"])` | (c) the same with `become` |
//! | `expand_match!([])` | (a) `ep9loop__match__run`: one `loop { match }` |
//! | `expand_scaled!([])` | `step::<S>`: one prefixed instruction, out of line |
//! | `handler!` | one handler fn with the handler ABI of the target (x86-64: `sysv64-unwind`) |
//!
//! Handler classes (read by `cargo xtask disasm-check`): `tail` handlers leave only through
//! a tail jump to the next handler; `exit` handlers (`Exit`, `BadOpcode`) return to Rust.

use ep9_bytecode::{OperandKind, OperandScale};

use crate::frame::HEADER;

// ---- operand scales ------------------------------------------------------------------------

/// An operand scale: bytes per scalable operand and the prefix length.
pub(crate) trait Scale {
    const SCALE: OperandScale;
    /// 0 without a prefix, else 1 (the `Wide`/`ExtraWide` byte before the opcode).
    const PREFIX: usize;
}

/// No prefix.
pub(crate) struct S1;
/// `Wide` prefix.
pub(crate) struct S2;
/// `ExtraWide` prefix.
pub(crate) struct S4;

impl Scale for S1 {
    const SCALE: OperandScale = OperandScale::Single;
    const PREFIX: usize = 0;
}
impl Scale for S2 {
    const SCALE: OperandScale = OperandScale::Wide;
    const PREFIX: usize = 1;
}
impl Scale for S4 {
    const SCALE: OperandScale = OperandScale::ExtraWide;
    const PREFIX: usize = 1;
}

/// Reads an unsigned scalable operand.
///
/// # Safety
/// `p` points at `S::SCALE.bytes()` readable bytes.
#[inline(always)]
unsafe fn read_unsigned<S: Scale>(p: *const u8) -> u32 {
    // SAFETY: caller contract; byte arrays have alignment 1.
    unsafe {
        match S::SCALE {
            OperandScale::Single => u32::from(*p),
            OperandScale::Wide => u32::from(u16::from_le_bytes(p.cast::<[u8; 2]>().read())),
            OperandScale::ExtraWide => u32::from_le_bytes(p.cast::<[u8; 4]>().read()),
        }
    }
}

/// Reads a signed scalable operand (sign extended).
///
/// # Safety
/// As for [`read_unsigned`].
#[inline(always)]
unsafe fn read_signed<S: Scale>(p: *const u8) -> i32 {
    // SAFETY: caller contract; byte arrays have alignment 1.
    unsafe {
        match S::SCALE {
            OperandScale::Single => i32::from(*p as i8),
            OperandScale::Wide => i32::from(i16::from_le_bytes(p.cast::<[u8; 2]>().read())),
            OperandScale::ExtraWide => i32::from_le_bytes(p.cast::<[u8; 4]>().read()),
        }
    }
}

// ---- typed operand values ------------------------------------------------------------------

/// A register as a slot offset from `fp` (H14 frame layout): local `r<v>` is `fp + v`, the
/// receiver (`v = -1`) is `fp - 9`, parameter `a<k>` (`v = -2 - k`) is `fp - 10 - k`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct R(pub isize);

impl R {
    /// Maps an encoded register operand to its slot offset, without a branch.
    #[inline(always)]
    pub const fn from_operand(v: i32) -> R {
        let o = v as isize;
        R(o + ((o >> (isize::BITS - 1)) & -(HEADER as isize)))
    }

    /// The register `i` places after this one (register pairs and triples hold locals).
    #[inline(always)]
    pub const fn plus(self, i: isize) -> R {
        R(self.0 + i)
    }
}

/// The first register of a register list (lists hold locals only, so this is `fp + first`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RList(pub isize);

/// A register count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Count(pub u32);

/// A constant pool index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct K(pub u32);

/// A feedback slot index (v0: feedback is not collected).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FSlot(pub u32);

/// A context slot index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CSlot(pub u32);

/// A context depth (hops outward).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Depth(pub u32);

/// A forward jump offset from the instruction start (prefix included).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fwd(pub u32);

/// A backward jump offset (`JumpLoop`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Back(pub u32);

/// A runtime function id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RtId(pub u16);

/// A `TestTypeOf` literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TyLit(pub u8);

/// Decoder for one operand kind.
pub(crate) trait Operand {
    const KIND: OperandKind;
    type V: Copy;
    /// # Safety
    /// `p` points at the operand's bytes in verified code.
    unsafe fn read<S: Scale>(p: *const u8) -> Self::V;
    #[inline(always)]
    fn size<S: Scale>() -> usize {
        Self::KIND.size(S::SCALE)
    }
}

/// Operand marker types, one per [`OperandKind`] variant (same names, so the opcode list's
/// kind tokens name them directly).
pub(crate) mod kind {
    use super::*;

    macro_rules! operand {
        ($name:ident, $ty:ty, |$p:ident, $S:ident| $read:expr) => {
            pub(crate) struct $name;
            impl Operand for $name {
                const KIND: OperandKind = OperandKind::$name;
                type V = $ty;
                #[inline(always)]
                unsafe fn read<$S: Scale>($p: *const u8) -> $ty {
                    // SAFETY: caller contract (the operand's bytes are readable).
                    unsafe { $read }
                }
            }
        };
    }

    operand!(Reg, R, |p, S| R::from_operand(read_signed::<S>(p)));
    operand!(RegOut, R, |p, S| R::from_operand(read_signed::<S>(p)));
    operand!(RegInOut, R, |p, S| R::from_operand(read_signed::<S>(p)));
    operand!(RegPair, R, |p, S| R::from_operand(read_signed::<S>(p)));
    operand!(RegOutTriple, R, |p, S| R::from_operand(read_signed::<S>(p)));
    operand!(RegOutPair, R, |p, S| R::from_operand(read_signed::<S>(p)));
    operand!(RegList, RList, |p, S| RList(read_signed::<S>(p) as isize));
    operand!(
        RegOutList,
        RList,
        |p, S| RList(read_signed::<S>(p) as isize)
    );
    operand!(RegCount, Count, |p, S| Count(read_unsigned::<S>(p)));
    operand!(Imm, i32, |p, S| read_signed::<S>(p));
    operand!(UImm, u32, |p, S| read_unsigned::<S>(p));
    operand!(Const, K, |p, S| K(read_unsigned::<S>(p)));
    operand!(Slot, FSlot, |p, S| FSlot(read_unsigned::<S>(p)));
    operand!(CtxSlot, CSlot, |p, S| CSlot(read_unsigned::<S>(p)));
    operand!(CtxDepth, Depth, |p, S| Depth(read_unsigned::<S>(p)));
    operand!(JumpFwd, Fwd, |p, S| Fwd(read_unsigned::<S>(p)));
    operand!(JumpBack, Back, |p, S| Back(read_unsigned::<S>(p)));
    operand!(Flag8, u8, |p, S| *p);
    operand!(Flag16, u16, |p, S| u16::from_le_bytes(
        p.cast::<[u8; 2]>().read()
    ));
    operand!(Runtime, RtId, |p, S| RtId(u16::from_le_bytes(
        p.cast::<[u8; 2]>().read()
    )));
    operand!(TypeOfLiteral, TyLit, |p, S| TyLit(*p));
}

/// Decodes the operands of the instruction at `$pc` (prefix included) at scale `$S`.
/// Evaluates to `(instruction length, (operands..))`. Must run inside `unsafe`.
macro_rules! decode {
    ($S:ty, $pc:expr; $($oname:ident : $okind:ident),*) => {{
        let mut at = <$S as $crate::interp::dsl::Scale>::PREFIX + 1;
        $(
            let $oname = <$crate::interp::dsl::kind::$okind as $crate::interp::dsl::Operand>
                ::read::<$S>($pc.add(at));
            at += <$crate::interp::dsl::kind::$okind as $crate::interp::dsl::Operand>::size::<$S>();
        )*
        (at, ($($oname,)*))
    }};
}
pub(crate) use decode;

// ---- handler signature rule (H15) ----------------------------------------------------------

/// A handler argument that fits in one integer register.
pub(crate) trait RegArg {}
impl RegArg for u64 {}
impl<T> RegArg for *const T {}
impl<T> RegArg for *mut T {}

/// Implemented only for handler types with at most 4 register-wide arguments, the first being
/// the context reference. Win64 passes 4 integer arguments in registers; a 5th goes on the
/// stack and can break the sibling call (RK25).
pub(crate) trait AtMostFourRegArgs {}
#[cfg(not(target_arch = "x86_64"))]
impl<C, A: RegArg, B: RegArg, D: RegArg, Ret: RegArg> AtMostFourRegArgs
    for for<'a> unsafe fn(&'a mut C, A, B, D) -> Ret
{
}
#[cfg(target_arch = "x86_64")]
impl<C, A: RegArg, B: RegArg, D: RegArg, Ret: RegArg> AtMostFourRegArgs
    for for<'a> unsafe extern "sysv64-unwind" fn(&'a mut C, A, B, D) -> Ret
{
}

/// Defines a handler (`fn name(cx, pc, fp, acc) { body }`, an `unsafe fn` of type
/// [`crate::interp::Handler`]) with the handler ABI of the target. x86-64: `extern
/// "sysv64-unwind"` on every OS, so Win64 handlers get the SysV register set (rsi, rdi and
/// xmm6-xmm15 are not callee-saved there) and save fewer registers on entry. Other targets: the
/// Rust ABI. `-unwind` keeps a panic (a VM bug) unwinding as with the Rust ABI.
macro_rules! handler {
    ( $(#[$($m:tt)*])* $vis:vis fn $name:ident($cx:ident, $pc:ident, $fp:ident, $acc:ident)
      $body:block ) => {
        #[cfg(target_arch = "x86_64")]
        $(#[$($m)*])*
        $vis unsafe extern "sysv64-unwind" fn $name(
            $cx: &mut $crate::interp::Cx,
            $pc: *const u8,
            $fp: *mut u64,
            $acc: u64,
        ) -> u64 $body

        #[cfg(not(target_arch = "x86_64"))]
        $(#[$($m)*])*
        $vis unsafe fn $name(
            $cx: &mut $crate::interp::Cx,
            $pc: *const u8,
            $fp: *mut u64,
            $acc: u64,
        ) -> u64 $body
    };
}
pub(crate) use handler;

const fn assert_handler_sig<T: AtMostFourRegArgs>() {}
const _: () = assert_handler_sig::<crate::interp::Handler>();

/// Debug builds have no tail-call optimization, so sibling dispatch grows the native stack by
/// one frame per instruction. Stop with a clear panic before the guard page.
///
/// The floor is `DEBUG_SLACK` below `native_limit`: a host call or JS entry that fails the
/// `native_limit` check throws a RangeError, and the handlers that unwind it need some stack
/// below that limit. `native_margin` is at least 128 KiB, so the floor stays at least
/// 64 KiB above the low end of the stack (room for the panic itself).
#[cfg(debug_assertions)]
#[inline(always)]
pub(crate) fn debug_native_stack_check(cx: &crate::interp::Cx) {
    const DEBUG_SLACK: usize = 64 << 10;
    assert!(
        ep9_platform::stack_pointer() > cx.rt().native_limit - DEBUG_SLACK,
        "ep9: the {:?} dispatch ran out of native stack (debug builds have no tail calls; \
         use Dispatch::Match)",
        cx.dispatch
    );
}

// ---- (b) and (c): threaded handlers --------------------------------------------------------

/// One handler per opcode, one cold handler per cold block, the `Exit`, `BadOpcode` and
/// `ConstructReturn` handlers, and the dispatch `TABLE`. `$tail` is `return` (sibling call,
/// checked by `cargo xtask disasm-check`) or `become` (guaranteed, nightly).
macro_rules! expand_threaded {
    ( [$tail:tt, $exp:literal]
      $( $(#[$doc:meta])* $name:ident = $code:literal, $snake:ident, acc: $acc:ident,
         flags: [$($flag:ident),*],
         operands: ($($oname:ident : $okind:ident $(($oparam:tt))?),*); )*
    ) => {
        $( $crate::interp::dsl::handler! {
            $(#[$doc])*
            ///
            /// # Safety
            /// Only the dispatch table calls this, with the state `interp::run` set up: `pc`
            /// is this opcode in verified code of the live top frame `fp`.
            #[unsafe(export_name = concat!("ep9h__", $exp, "__tail__", stringify!($name)))]
            #[allow(non_snake_case, unused_mut, unused_variables, unused_unsafe, unreachable_code)]
            pub(crate) fn $name(cx, pc, fp, acc) {
                #[cfg(debug_assertions)]
                $crate::interp::dsl::debug_native_stack_check(cx);
                // SAFETY: the verifier checked every operand of this instruction (registers in
                // the frame, indexes in their tables, jump targets on instruction starts).
                let (len, ($($oname,)*)) = unsafe {
                    $crate::interp::dsl::decode!($crate::interp::dsl::S1, pc; $($oname: $okind),*)
                };
                // SAFETY: as above; a verified function never falls off its end.
                let mut st = unsafe { St::new(cx, pc, fp, acc, pc.add(len)) };
                let next: *const u8 = $crate::interp::ops::if_cold!($name {
                    match $crate::interp::ops::$name::fast(&mut st, $($oname),*) {
                        Flow::Next => st.next,
                        Flow::Jump(t) => t,
                        Flow::Cold => {
                            let St { cx, pc, fp, acc, .. } = st;
                            // SAFETY: the cold handler takes the same state as this one.
                            unsafe { $tail cold::$name(cx, pc, fp, acc) }
                        }
                    }
                } else {
                    match $crate::interp::ops::$name::fast(&mut st, $($oname),*) {
                        Done::Next => st.next,
                        Done::Jump(t) => t,
                    }
                });
                let St { cx, fp, acc, .. } = st;
                // SAFETY: `next` is an instruction start of verified code or a VM stub.
                let h = cx.table[unsafe { *next } as usize];
                // SAFETY: the table holds handlers of this expansion; the state is valid.
                unsafe { $tail h(cx, next, fp, acc) }
            }
        } )*

        /// Cold handlers: one per opcode with a cold block (runtime calls live here).
        pub(crate) mod cold {
            #[allow(unused_imports)]
            use super::*;
            $(
                $crate::interp::ops::if_cold!($name { $crate::interp::dsl::handler! {
                    /// Cold path of the opcode with the same name.
                    ///
                    /// # Safety
                    /// As for the fast handler, which is the only caller.
                    #[unsafe(export_name = concat!("ep9h__", $exp, "__tail__", stringify!($name), "_cold"))]
                    #[cold]
                    #[inline(never)]
                    #[allow(non_snake_case, unused_mut, unused_variables, unused_unsafe)]
                    pub(crate) fn $name(cx, pc, fp, acc) {
                        // SAFETY: as in the fast handler.
                        let (len, ($($oname,)*)) = unsafe {
                            $crate::interp::dsl::decode!($crate::interp::dsl::S1, pc; $($oname: $okind),*)
                        };
                        // SAFETY: as in the fast handler.
                        let mut st = unsafe { St::new(cx, pc, fp, acc, pc.add(len)) };
                        let next = match $crate::interp::ops::$name::cold(&mut st, $($oname),*) {
                            Done::Next => st.next,
                            Done::Jump(t) => t,
                        };
                        let St { cx, fp, acc, .. } = st;
                        // SAFETY: as in the fast handler.
                        let h = cx.table[unsafe { *next } as usize];
                        // SAFETY: as in the fast handler.
                        unsafe { $tail h(cx, next, fp, acc) }
                    }
                } } else {});
            )*
        }

        $crate::interp::dsl::handler! {
            /// Leaves the dispatch: returns the accumulator (0 = exception pending) to Rust.
            ///
            /// # Safety
            /// Only the dispatch table calls this.
            #[unsafe(export_name = concat!("ep9h__", $exp, "__exit__Exit"))]
            #[allow(unused_variables)]
            fn exit(cx, pc, fp, acc) {
                acc
            }
        }

        $crate::interp::dsl::handler! {
            /// A byte that is no opcode (a VM bug: the verifier accepts defined opcodes only).
            ///
            /// # Safety
            /// Only the dispatch table calls this.
            #[unsafe(export_name = concat!("ep9h__", $exp, "__exit__BadOpcode"))]
            #[allow(unused_variables)]
            fn bad_opcode(cx, pc, fp, acc) {
                $crate::interp::slow::bad_opcode(cx, fp, pc)
            }
        }

        $crate::interp::dsl::handler! {
            /// The `[[Construct]]` result check of a construct stub frame (H14).
            ///
            /// # Safety
            /// Only the dispatch table calls this, with `fp` a construct stub frame.
            #[unsafe(export_name = concat!("ep9h__", $exp, "__tail__ConstructReturn"))]
            #[allow(unused_variables)]
            fn construct_return(cx, pc, fp, acc) {
                // SAFETY: caller contract.
                let (next, fp, acc) = unsafe { $crate::interp::construct_return(fp, acc) };
                // SAFETY: `next` is the caller's next instruction.
                let h = cx.table[unsafe { *next } as usize];
                // SAFETY: as in the opcode handlers.
                unsafe { $tail h(cx, next, fp, acc) }
            }
        }

        /// The dispatch table of this expansion.
        pub(crate) static TABLE: [Handler; 256] = {
            let mut t = [bad_opcode as Handler; 256];
            $( t[$code] = $name as Handler; )*
            t[OP_EXIT as usize] = exit as Handler;
            t[OP_CONSTRUCT_RETURN as usize] = construct_return as Handler;
            t
        };
    };
}
pub(crate) use expand_threaded;

// ---- (a) the match loop --------------------------------------------------------------------

/// One `loop { match }` function. Cold blocks run in out-of-line functions (`cold::*`), so
/// each arm stays small. LLVM does not copy the dispatch into the arms here (one dispatch jump
/// on every target, H15 2026-10-21): the reachable `_` arm keeps a jump-table range check that
/// splits the dispatch block, and the dispatch block has too many predecessors and successors
/// for early tail duplication.
macro_rules! expand_match {
    ( []
      $( $(#[$doc:meta])* $name:ident = $code:literal, $snake:ident, acc: $acc:ident,
         flags: [$($flag:ident),*],
         operands: ($($oname:ident : $okind:ident $(($oparam:tt))?),*); )*
    ) => {
        /// Runs from `pc` until an `Exit`. Exported as `ep9loop__match__run`, so
        /// `cargo xtask disasm-check --match-loop` can print its shape. It is the debug
        /// expansion; release builds use sibling calls on every target (H15, 2026-10-21).
        ///
        /// # Safety
        /// `pc` and `fp` come from `interp::run`: `pc` is an instruction of verified code in
        /// the live top frame `fp`.
        #[inline(never)]
        #[unsafe(export_name = "ep9loop__match__run")]
        #[allow(unused_mut, unused_variables, unused_unsafe, unreachable_code)]
        pub(crate) unsafe fn run(cx: &mut Cx, pc: *const u8, fp: *mut u64, acc: u64) -> u64 {
            let mut pc = pc;
            let mut fp = fp;
            let mut acc = acc;
            loop {
                // SAFETY: `pc` is an instruction start of verified code or a VM stub.
                let op = unsafe { *pc };
                match op {
                    $( $code => {
                        // SAFETY: as in the threaded handlers.
                        let (len, ($($oname,)*)) = unsafe {
                            $crate::interp::dsl::decode!($crate::interp::dsl::S1, pc; $($oname: $okind),*)
                        };
                        // SAFETY: as in the threaded handlers.
                        let mut st = unsafe { St::new(&mut *cx, pc, fp, acc, pc.add(len)) };
                        $crate::interp::ops::if_cold!($name {
                            match $crate::interp::ops::$name::fast(&mut st, $($oname),*) {
                                Flow::Next => {
                                    pc = st.next;
                                    fp = st.fp;
                                    acc = st.acc;
                                }
                                Flow::Jump(t) => {
                                    pc = t;
                                    fp = st.fp;
                                    acc = st.acc;
                                }
                                Flow::Cold => {
                                    let (f, a) = (st.fp, st.acc);
                                    // SAFETY: the cold function takes the same state as this arm.
                                    acc = unsafe { cold::$name(&mut *cx, pc, f, a) };
                                    fp = cx.out_fp;
                                    pc = cx.out_pc;
                                }
                            }
                        } else {
                            pc = match $crate::interp::ops::$name::fast(&mut st, $($oname),*) {
                                Done::Next => st.next,
                                Done::Jump(t) => t,
                            };
                            fp = st.fp;
                            acc = st.acc;
                        });
                    } )*
                    OP_EXIT => return acc,
                    OP_CONSTRUCT_RETURN => {
                        // SAFETY: only a construct stub frame returns to this stub.
                        let (next, f, a) = unsafe { $crate::interp::construct_return(fp, acc) };
                        pc = next;
                        fp = f;
                        acc = a;
                    }
                    _ => return $crate::interp::slow::bad_opcode(cx, fp, pc),
                }
            }
        }

        /// Cold blocks of the match loop: one function per opcode with a cold block. Each
        /// returns the accumulator and sets `cx.out_fp`/`cx.out_pc` (the next state).
        mod cold {
            #[allow(unused_imports)]
            use super::*;
            $(
                $crate::interp::ops::if_cold!($name {
                    /// # Safety
                    /// As for the arm of `run` with the same opcode, which is the only caller.
                    #[cold]
                    #[inline(never)]
                    #[allow(non_snake_case, unused_mut, unused_variables, unused_unsafe)]
                    pub(super) unsafe fn $name(cx: &mut Cx, pc: *const u8, fp: *mut u64, acc: u64) -> u64 {
                        // SAFETY: as in the arm.
                        let (len, ($($oname,)*)) = unsafe {
                            $crate::interp::dsl::decode!($crate::interp::dsl::S1, pc; $($oname: $okind),*)
                        };
                        // SAFETY: as in the arm.
                        let mut st = unsafe { St::new(&mut *cx, pc, fp, acc, pc.add(len)) };
                        let next = match $crate::interp::ops::$name::cold(&mut st, $($oname),*) {
                            Done::Next => st.next,
                            Done::Jump(t) => t,
                        };
                        let (f, a) = (st.fp, st.acc);
                        cx.out_fp = f;
                        cx.out_pc = next;
                        a
                    }
                } else {});
            )*
        }
    };
}
pub(crate) use expand_match;

// ---- prefixed instructions -----------------------------------------------------------------

/// `step::<S>`: runs the one instruction at `pc` (a `Wide`/`ExtraWide` prefix) with operands
/// at scale `S`. Returns the accumulator and sets `cx.out_fp`/`cx.out_pc` (the next state).
macro_rules! expand_scaled {
    ( []
      $( $(#[$doc:meta])* $name:ident = $code:literal, $snake:ident, acc: $acc:ident,
         flags: [$($flag:ident),*],
         operands: ($($oname:ident : $okind:ident $(($oparam:tt))?),*); )*
    ) => {
        /// # Safety
        /// `pc` points at a prefix byte of verified code in the live top frame `fp`.
        #[inline(never)]
        #[allow(unused_mut, unused_variables, unused_unsafe, unreachable_code)]
        pub(crate) unsafe fn step<S: $crate::interp::dsl::Scale>(
            cx: &mut Cx,
            pc: *const u8,
            fp: *mut u64,
            acc: u64,
        ) -> u64 {
            // SAFETY: a verified prefix is followed by an opcode byte.
            let op = unsafe { *pc.add(1) };
            match op {
                $( $code if const { ep9_bytecode::Opcode::$name.info().has_scalable_operands() } => {
                    // SAFETY: as in the threaded handlers, at scale `S`.
                    let (len, ($($oname,)*)) = unsafe {
                        $crate::interp::dsl::decode!(S, pc; $($oname: $okind),*)
                    };
                    // SAFETY: as in the threaded handlers.
                    let mut st = unsafe { St::new(&mut *cx, pc, fp, acc, pc.add(len)) };
                    let done: Done = $crate::interp::ops::if_cold!($name {
                        match $crate::interp::ops::$name::fast(&mut st, $($oname),*) {
                            Flow::Next => Done::Next,
                            Flow::Jump(t) => Done::Jump(t),
                            Flow::Cold => $crate::interp::ops::$name::cold(&mut st, $($oname),*),
                        }
                    } else {
                        $crate::interp::ops::$name::fast(&mut st, $($oname),*)
                    });
                    let next = match done {
                        Done::Next => st.next,
                        Done::Jump(t) => t,
                    };
                    let (f, a) = (st.fp, st.acc);
                    cx.out_fp = f;
                    cx.out_pc = next;
                    a
                } )*
                _ => {
                    let r = $crate::interp::slow::bad_opcode(cx, fp, pc);
                    cx.out_fp = fp;
                    cx.out_pc = $crate::interp::EXIT_STUB.as_ptr();
                    r
                }
            }
        }
    };
}
pub(crate) use expand_scaled;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_mapping_follows_the_frame_layout() {
        assert_eq!(R::from_operand(0).0, 0);
        assert_eq!(R::from_operand(5).0, 5);
        // The receiver is fp - 9, parameter a<k> is fp - 10 - k (H14).
        assert_eq!(R::from_operand(-1).0, crate::frame::slot::RECEIVER);
        assert_eq!(R::from_operand(-2).0, crate::frame::slot::PARAM0);
        assert_eq!(R::from_operand(-2 - 7).0, crate::frame::slot::PARAM0 - 7);
    }

    #[test]
    fn operands_decode_at_every_scale() {
        let b = [0x80u8, 0xff, 0xff, 0x7f];
        // SAFETY: `b` has 4 readable bytes.
        unsafe {
            assert_eq!(<kind::Imm as Operand>::read::<S1>(b.as_ptr()), -128);
            assert_eq!(<kind::UImm as Operand>::read::<S1>(b.as_ptr()), 0x80);
            assert_eq!(<kind::Imm as Operand>::read::<S2>(b.as_ptr()), -128);
            assert_eq!(<kind::UImm as Operand>::read::<S2>(b.as_ptr()), 0xff80);
            assert_eq!(<kind::Imm as Operand>::read::<S4>(b.as_ptr()), 0x7fff_ff80);
            assert_eq!(<kind::Flag16 as Operand>::read::<S4>(b.as_ptr()), 0xff80);
            assert_eq!(<kind::Runtime as Operand>::read::<S1>(b.as_ptr()).0, 0xff80);
        }
        assert_eq!(<kind::Flag8 as Operand>::size::<S4>(), 1);
        assert_eq!(<kind::Reg as Operand>::size::<S4>(), 4);
        assert_eq!(<kind::Runtime as Operand>::size::<S1>(), 2);
    }
}
