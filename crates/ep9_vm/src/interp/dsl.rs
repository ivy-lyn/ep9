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
//! | `expand_scaled!([])` | `step::<S>`: one prefixed instruction, out of line; its cold blocks in `step_cold::<S>` |
//! | `handler!` | one handler fn with the handler ABI of the target (x86-64: `sysv64-unwind`; `sysv64` under `panic = "abort"`) |
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

    operand!(Reg, R, |p, S| R::of_operand(read_signed::<S>(p)));
    operand!(RegOut, R, |p, S| R::of_operand(read_signed::<S>(p)));
    operand!(RegInOut, R, |p, S| R::of_operand(read_signed::<S>(p)));
    operand!(RegPair, R, |p, S| R::of_operand(read_signed::<S>(p)));
    operand!(RegOutTriple, R, |p, S| R::of_operand(read_signed::<S>(p)));
    operand!(RegOutPair, R, |p, S| R::of_operand(read_signed::<S>(p)));
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
#[cfg(all(target_arch = "x86_64", panic = "unwind"))]
impl<C, A: RegArg, B: RegArg, D: RegArg, Ret: RegArg> AtMostFourRegArgs
    for for<'a> unsafe extern "sysv64-unwind" fn(&'a mut C, A, B, D) -> Ret
{
}
#[cfg(all(target_arch = "x86_64", not(panic = "unwind")))]
impl<C, A: RegArg, B: RegArg, D: RegArg, Ret: RegArg> AtMostFourRegArgs
    for for<'a> unsafe extern "sysv64" fn(&'a mut C, A, B, D) -> Ret
{
}

/// Defines a handler (`fn name(cx, pc, fp, acc) { body }`, an `unsafe fn` of type
/// [`crate::interp::Handler`]) with the handler ABI of the target. x86-64: `extern
/// "sysv64-unwind"` on every OS, so Win64 handlers get the SysV register set (rsi, rdi and
/// xmm6-xmm15 are not callee-saved there) and save fewer registers on entry. Other targets: the
/// Rust ABI. `-unwind` keeps a panic (a VM bug) unwinding as with the Rust ABI. With `panic =
/// "abort"` (the release profile, H23) the ABI is plain `sysv64`: nothing unwinds there, and a
/// call to an `-unwind` function from abort code gets an abort landing pad, which turns every
/// sibling call into a call + return (disasm-check: 50 of 282 tail jumps).
macro_rules! handler {
    ( $(#[$($m:tt)*])* $vis:vis fn $name:ident($cx:ident, $pc:ident, $fp:ident, $acc:ident)
      $body:block ) => {
        #[cfg(all(target_arch = "x86_64", panic = "unwind"))]
        $(#[$($m)*])*
        $vis unsafe extern "sysv64-unwind" fn $name(
            $cx: &mut $crate::interp::Cx,
            $pc: *const u8,
            $fp: *mut u64,
            $acc: u64,
        ) -> u64 $body

        #[cfg(all(target_arch = "x86_64", not(panic = "unwind")))]
        $(#[$($m)*])*
        $vis unsafe extern "sysv64" fn $name(
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

/// The `Flow::Record` arm of a threaded fast handler. (b) (`return`): a tail call of
/// [`record_next`] with the handler's state (the instruction's own pc) and the record's bits;
/// its const parameter is the instruction's [`slot_place`] (the slot operand's offset and the
/// instruction length), so the handler holds no value for the slot: `record_next` reads the
/// slot operand at its constant offset and finds the next pc. (c) (`become` needs the same
/// signature): records
/// inline and continues at the next pc. The `ics_only` expansions (MODES-D11, MODES-D19; M5-P3
/// PROFILE6) drop the record: (b) tail-calls [`skip_next`] (no record, the next pc), (c)
/// continues at the next pc. Their fast handlers have no record code; the bits are dead there.
macro_rules! record_exit {
    (return, ics_only, $name:ident, $st:ident, $r:ident) => {{
        let _ = $r;
        $crate::interp::dsl::skip_exit!($name, $st)
    }};
    (become, ics_only, $name:ident, $st:ident, $r:ident) => {{
        let _ = $r;
        $st.next
    }};
    (return, full, $name:ident, $st:ident, $r:ident) => {{
        let St {
            cx, pc, fp, acc, ..
        } = $st;
        // SAFETY: `pc` is this one-byte-scale instruction of verified code in the live frame
        // `fp`; `slot_place` gives its slot operand and its length.
        unsafe {
            return $crate::interp::dsl::record_next::<
                { $crate::interp::dsl::slot_place(ep9_bytecode::Opcode::$name) },
            >(cx, pc, fp, acc, $r.bits());
        }
    }};
    (become, full, $name:ident, $st:ident, $r:ident) => {{
        $crate::profile::record(&$st, $r);
        $st.next
    }};
}
pub(crate) use record_exit;

/// The record arms of an `ics_only` sibling handler: a tail call of [`skip_next`] with the
/// handler's state and the instruction length, no record.
macro_rules! skip_exit {
    ($name:ident, $st:ident) => {{
        let St {
            cx, pc, fp, acc, ..
        } = $st;
        // SAFETY: `pc` is this one-byte-scale instruction of verified code in the live frame
        // `fp`; `one_byte_len` is its length.
        unsafe {
            return $crate::interp::dsl::skip_next::<
                { $crate::interp::dsl::one_byte_len(ep9_bytecode::Opcode::$name) },
            >(cx, pc, fp, acc);
        }
    }};
}
pub(crate) use skip_exit;

/// The `Flow::RecordPair` arm of a threaded fast handler: as [`record_exit`], with the operand
/// pair as the last two arguments of [`record_pair_next`] and the slot place as its const
/// parameter. `ics_only`: no record, as in [`record_exit`].
macro_rules! record_pair_exit {
    (return, ics_only, $name:ident, $st:ident, $p:ident) => {{
        let _ = $p;
        $crate::interp::dsl::skip_exit!($name, $st)
    }};
    (become, ics_only, $name:ident, $st:ident, $p:ident) => {{
        let _ = $p;
        $st.next
    }};
    (return, full, $name:ident, $st:ident, $p:ident) => {{
        let St {
            cx, pc, fp, acc, ..
        } = $st;
        let (l, r) = $p.operands();
        // SAFETY: as in `record_exit`.
        unsafe {
            return $crate::interp::dsl::record_pair_next::<
                { $crate::interp::dsl::slot_place(ep9_bytecode::Opcode::$name) },
            >(cx, pc, fp, acc, l, r);
        }
    }};
    (become, full, $name:ident, $st:ident, $p:ident) => {{
        $crate::profile::record_pair(&$st, $p);
        $st.next
    }};
}
pub(crate) use record_pair_exit;

/// The `Flow::JumpRecordCall` arm of a threaded fast handler: (b) tail-calls
/// [`record_call_next`] with the callee's state (the handler signature: the target pc, the new
/// frame) and the slot place of the caller's instruction as its const parameter; (c) records
/// inline and jumps. `ics_only`: no record, the jump to the callee.
macro_rules! record_call_exit {
    ($tail:tt, ics_only, $name:ident, $st:ident, $t:ident, $s:ident) => {{
        let _ = $s;
        $t
    }};
    (return, full, $name:ident, $st:ident, $t:ident, $s:ident) => {{
        let St { cx, fp, acc, .. } = $st;
        // SAFETY: `fp` is the frame `call_fast` pushed and `$t` its first pc (or a VM stub);
        // its return pc is the next instruction of the caller.
        unsafe {
            return $crate::interp::dsl::record_call_next::<
                { $crate::interp::dsl::slot_back(ep9_bytecode::Opcode::$name) as usize },
            >(cx, $t, fp, acc);
        }
    }};
    (become, full, $name:ident, $st:ident, $t:ident, $s:ident) => {{
        $crate::profile::record_call_pushed(&$st, $s);
        $t
    }};
}
pub(crate) use record_call_exit;

/// Defines [`record_call_next`] with the handler ABI (`handler!`) and a const parameter.
macro_rules! record_call_next_fn {
    ($($abi:literal => $cfg:meta;)*) => {$(
        /// The record of a `Flow::JumpRecordCall` (sibling expansion): reads the caller's
        /// Call slot operand `BACK` bytes before the return pc of the new frame `fp`, runs
        /// `profile::record_call_pushed`, then dispatches `pc` in the new frame. It has the
        /// handler signature (it gets the callee's state) but is not in the table;
        /// `cargo xtask disasm-check` follows the jump to it as part of the handler.
        ///
        /// # Safety
        /// `fp` is the frame `call_fast` just pushed, `pc` the instruction (or VM stub) the
        /// dispatch runs next in it, and the caller's instruction ends at the frame's return
        /// pc with a one-byte slot operand `BACK` bytes before it.
        #[cfg($cfg)]
        #[inline(never)]
        pub(crate) unsafe extern $abi fn record_call_next<const BACK: usize>(
            cx: &mut $crate::interp::Cx,
            pc: *const u8,
            fp: *mut u64,
            acc: u64,
        ) -> u64 {
            #[cfg(debug_assertions)]
            debug_native_stack_check(cx);
            debug_assert!(BACK != 0, "Flow::JumpRecordCall from an opcode without a slot operand");
            // SAFETY: caller contract: the return pc slot of the new frame holds the caller's
            // next instruction as a native word.
            let ret = crate::frame::native_ptr::<u8>(unsafe {
                *fp.offset(crate::frame::slot::RETURN_PC)
            });
            // SAFETY: caller contract.
            let slot = FSlot(u32::from(unsafe { *ret.sub(BACK) }));
            // SAFETY: caller contract (`St::new`: `pc` is an instruction start of `fp`).
            let st = unsafe { crate::interp::St::new(cx, pc, fp, acc, pc) };
            $crate::profile::record_call_pushed(&st, slot);
            let crate::interp::St { cx, fp, acc, .. } = st;
            // SAFETY: `pc` is an instruction start of verified code or a VM stub.
            let h = cx.table[unsafe { *pc } as usize];
            // SAFETY: the table holds handlers of the sibling expansion; the state is valid.
            unsafe { h(cx, pc, fp, acc) }
        }
    )*};
}

record_call_next_fn! {
    "sysv64-unwind" => all(target_arch = "x86_64", panic = "unwind");
    "sysv64" => all(target_arch = "x86_64", not(panic = "unwind"));
    "Rust" => not(target_arch = "x86_64");
}

/// For an opcode with a feedback slot operand, at one-byte scale (no prefix): the operand's
/// offset from the opcode byte (bits 0-7) and the instruction length (bits 8-15), for
/// [`record_next`] and [`record_pair_next`]. 0 for an opcode without one (it never returns a
/// record flow).
pub(crate) const fn slot_place(op: ep9_bytecode::Opcode) -> u64 {
    let info = op.info();
    let mut i = 0;
    while i < info.operands.len() {
        if matches!(info.operands[i].kind, OperandKind::Slot) {
            assert!(info.operands[i].kind.size(OperandScale::Single) == 1);
            let off = info.operand_offset(i, OperandScale::Single);
            let len = info.size(OperandScale::Single);
            assert!(off < 256 && len < 256);
            return (off | (len << 8)) as u64;
        }
        i += 1;
    }
    0
}

/// For an opcode with a feedback slot operand: the distance in bytes from the operand to the
/// end of the instruction at one-byte scale (no prefix), so [`record_call_next`] finds it from
/// the return pc. 0 for an opcode without one.
pub(crate) const fn slot_back(op: ep9_bytecode::Opcode) -> u64 {
    let info = op.info();
    let mut i = 0;
    while i < info.operands.len() {
        if matches!(info.operands[i].kind, OperandKind::Slot) {
            assert!(info.operands[i].kind.size(OperandScale::Single) == 1);
            let back =
                info.size(OperandScale::Single) - info.operand_offset(i, OperandScale::Single);
            return back as u64;
        }
        i += 1;
    }
    0
}

/// Defines [`record_next`] with the ABI of the handlers (`handler!`) plus a 5th integer
/// argument, which every handler ABI passes in a register (SysV: r8; AArch64: x4), so the
/// handler's tail call to it stays a sibling call.
macro_rules! record_next_fn {
    ($($abi:literal => $cfg:meta;)*) => {$(
        /// The record of a `Flow::Record` (sibling expansion): runs `profile::record` for the
        /// frame `fp`, then dispatches the next instruction as the handler would have.
        /// Out of line (a fast handler reaches it by a tail call), so on x86-64 the record's
        /// registers are saved here, not on entry to every handler that may record
        /// (M4-P1 PROFILE-X64). One copy per `PLACE` (M7-P2 RECCOST): the slot operand's
        /// offset and the instruction length are constants, so it decodes no word (x86-64: 16
        /// instructions on the path that adds no bit, 22 before). `cargo xtask disasm-check`
        /// follows the jump to it as part of the handler.
        ///
        /// # Safety
        /// `pc` is an instruction of verified code at one-byte scale in the live top frame
        /// `fp`, `PLACE` is its `slot_place`, and `bits` is a `profile::Rec::bits`.
        #[cfg($cfg)]
        #[inline(never)]
        pub(crate) unsafe extern $abi fn record_next<const PLACE: u64>(
            cx: &mut $crate::interp::Cx,
            pc: *const u8,
            fp: *mut u64,
            acc: u64,
            bits: u64,
        ) -> u64 {
            #[cfg(debug_assertions)]
            debug_native_stack_check(cx);
            let (off, len) = ((PLACE & 0xFF) as usize, ((PLACE >> 8) & 0xFF) as usize);
            debug_assert!(off != 0, "Flow::Record from an opcode without a slot operand");
            // SAFETY: caller contract: the one-byte slot operand is at `pc + off` and the
            // instruction is `len` bytes long, in verified code.
            let (slot, next) = unsafe { (u32::from(*pc.add(off)), pc.add(len)) };
            let slot = $crate::interp::dsl::FSlot(slot);
            // SAFETY: caller contract (`St::new`: `next` is an instruction start of `fp`).
            let st = unsafe { crate::interp::St::new(cx, next, fp, acc, next) };
            $crate::profile::record(&st, $crate::profile::Rec::new(slot, bits as u32));
            let crate::interp::St { cx, fp, acc, .. } = st;
            // SAFETY: `next` is an instruction start of verified code.
            let h = cx.table[unsafe { *next } as usize];
            // SAFETY: the table holds handlers of the sibling expansion; the state is valid.
            unsafe { h(cx, next, fp, acc) }
        }
    )*};
}

/// Defines [`record_pair_next`] as [`record_next_fn`] does `record_next`, with 6 integer
/// arguments (SysV: rdi, rsi, rdx, rcx, r8, r9; AArch64: x0-x5), all in registers.
macro_rules! record_pair_next_fn {
    ($($abi:literal => $cfg:meta;)*) => {$(
        /// The record of a `Flow::RecordPair` (sibling expansion): `profile::record_compare`
        /// of the pair `l`, `r` for the slot operand of the instruction at `pc` (`PLACE`, a
        /// `slot_place`), then the dispatch of the next instruction. Out of line for the
        /// reason of `record_next`.
        ///
        /// # Safety
        /// `pc` is an instruction of verified code at one-byte scale in the live top frame
        /// `fp`, and `PLACE` is its `slot_place`.
        #[cfg($cfg)]
        #[inline(never)]
        pub(crate) unsafe extern $abi fn record_pair_next<const PLACE: u64>(
            cx: &mut $crate::interp::Cx,
            pc: *const u8,
            fp: *mut u64,
            acc: u64,
            l: u64,
            r: u64,
        ) -> u64 {
            #[cfg(debug_assertions)]
            debug_native_stack_check(cx);
            let (off, len) = ((PLACE & 0xFF) as usize, ((PLACE >> 8) & 0xFF) as usize);
            debug_assert!(off != 0, "Flow::RecordPair from an opcode without a slot operand");
            // SAFETY: caller contract.
            let (slot, next) = unsafe { (u32::from(*pc.add(off)), pc.add(len)) };
            let slot = $crate::interp::dsl::FSlot(slot);
            // SAFETY: caller contract (`St::new`: `next` is an instruction start of `fp`).
            let st = unsafe { crate::interp::St::new(cx, next, fp, acc, next) };
            $crate::profile::record_pair(&st, $crate::profile::Pair::new(slot, l, r));
            let crate::interp::St { cx, fp, acc, .. } = st;
            // SAFETY: `next` is an instruction start of verified code.
            let h = cx.table[unsafe { *next } as usize];
            // SAFETY: the table holds handlers of the sibling expansion; the state is valid.
            unsafe { h(cx, next, fp, acc) }
        }
    )*};
}

record_pair_next_fn! {
    "sysv64-unwind" => all(target_arch = "x86_64", panic = "unwind");
    "sysv64" => all(target_arch = "x86_64", not(panic = "unwind"));
    "Rust" => not(target_arch = "x86_64");
}

record_next_fn! {
    "sysv64-unwind" => all(target_arch = "x86_64", panic = "unwind");
    "sysv64" => all(target_arch = "x86_64", not(panic = "unwind"));
    "Rust" => not(target_arch = "x86_64");
}

/// Defines [`skip_next`] with the handler ABI (`handler!`) and a const parameter.
macro_rules! skip_next_fn {
    ($($abi:literal => $cfg:meta;)*) => {$(
        /// The `Flow::Record` and `Flow::RecordPair` arms of an `ics_only` sibling expansion
        /// (MODES-D11): no record, the dispatch of the instruction `LEN` bytes after `pc`.
        /// Out of line, as [`record_next`] is for the `full` table, so a fast handler keeps the
        /// shape of its `full` twin: LLVM does not merge a rare record path (a double result,
        /// a compare of two non-int32 values) into the int32 path with selects, which cost
        /// the int32 path a few instructions (t0-loop +1.9% on M5-P3 a64 before this helper).
        /// `cargo xtask disasm-check` follows the jump to it as part of the handler.
        ///
        /// # Safety
        /// `pc` is an instruction of verified code at one-byte scale in the live top frame
        /// `fp`, `LEN` bytes long.
        #[cfg($cfg)]
        #[inline(never)]
        pub(crate) unsafe extern $abi fn skip_next<const LEN: usize>(
            cx: &mut $crate::interp::Cx,
            pc: *const u8,
            fp: *mut u64,
            acc: u64,
        ) -> u64 {
            #[cfg(debug_assertions)]
            debug_native_stack_check(cx);
            // SAFETY: caller contract: the next instruction starts `LEN` bytes after `pc`.
            let next = unsafe { pc.add(LEN) };
            // SAFETY: `next` is an instruction start of verified code.
            let h = cx.table[unsafe { *next } as usize];
            // SAFETY: the table holds handlers of the sibling expansion; the state is valid.
            unsafe { h(cx, next, fp, acc) }
        }
    )*};
}

skip_next_fn! {
    "sysv64-unwind" => all(target_arch = "x86_64", panic = "unwind");
    "sysv64" => all(target_arch = "x86_64", not(panic = "unwind"));
    "Rust" => not(target_arch = "x86_64");
}

/// The instruction length of `op` at one-byte scale, for [`skip_next`].
pub(crate) const fn one_byte_len(op: ep9_bytecode::Opcode) -> usize {
    op.info().size(OperandScale::Single)
}

/// The cold handler of opcode `$name` in an [`expand_threaded`] expansion. `full`: the cold
/// body (`ops::<Name>::cold`), then the dispatch. `ics_only`: a tail jump to the cold handler
/// of the `full` table of the same `$tail` (`interp::sibling` or `interp::tail`): the cold
/// bodies are the same code in both modes (their record hooks test the mode,
/// `profile::records`), so the `ics_only` table adds no copy of them.
macro_rules! cold_handler {
    ([$tail:tt, $exp:literal, full] $name:ident ($($oname:ident : $okind:ident),*)) => {
        $crate::interp::dsl::handler! {
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
        }
    };
    ([return, $exp:literal, ics_only] $name:ident ($($oname:ident : $okind:ident),*)) => {
        $crate::interp::dsl::cold_handler!(@forward [return, $exp] $name, $crate::interp::sibling::cold::$name);
    };
    ([become, $exp:literal, ics_only] $name:ident ($($oname:ident : $okind:ident),*)) => {
        $crate::interp::dsl::cold_handler!(@forward [become, $exp] $name, $crate::interp::tail::cold::$name);
    };
    (@forward [$tail:tt, $exp:literal] $name:ident, $full:path) => {
        $crate::interp::dsl::handler! {
            /// Cold path of the opcode with the same name: the cold handler of the `full`
            /// table (the same state; it dispatches through `cx.table`, so it continues in
            /// this table).
            ///
            /// # Safety
            /// As for the fast handler, which is the only caller.
            #[unsafe(export_name = concat!("ep9h__", $exp, "__tail__", stringify!($name), "_cold"))]
            #[cold]
            #[inline(never)]
            #[allow(non_snake_case)]
            pub(crate) fn $name(cx, pc, fp, acc) {
                // SAFETY: the same handler contract.
                unsafe { $tail $full(cx, pc, fp, acc) }
            }
        }
    };
}
pub(crate) use cold_handler;

/// One handler per opcode, one cold handler per cold block, the `Exit`, `BadOpcode` and
/// `ConstructReturn` handlers, and the dispatch `TABLE`. `$tail` is `return` (sibling call,
/// checked by `cargo xtask disasm-check`) or `become` (guaranteed, nightly).
///
/// `$mode` is the type-feedback mode of the table (MODES-D11; M5-P3 PROFILE6):
///
/// | `$mode` | Fast handlers | Cold handlers |
/// |---|---|---|
/// | `full` | `Flow::Record`, `RecordPair`, `JumpRecordCall` run the profile record ([`record_exit`]) | the cold bodies ([`cold_handler`]) |
/// | `ics_only` | the same arms continue with no record (sibling: a tail call of [`skip_next`]): no `profile.rs` record code in a fast handler | one tail jump to the cold handler of the `full` table of the same `$tail` ([`cold_handler`]) |
///
/// The record hooks of the cold bodies are `ops.rs` code that both tables share, so they test
/// the isolate's mode themselves (`profile::records`, on the cold path only). Every expansion
/// has the same handler count (`ep9_vm::internals::THREADED_HANDLERS`, the
/// `cargo xtask disasm-check --expect` value).
macro_rules! expand_threaded {
    ( [$tail:tt, $exp:literal, $mode:ident]
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
                        Flow::Record(r) => $crate::interp::dsl::record_exit!($tail, $mode, $name, st, r),
                        Flow::RecordPair(p) => {
                            $crate::interp::dsl::record_pair_exit!($tail, $mode, $name, st, p)
                        }
                        Flow::JumpRecordCall(t, s) => {
                            $crate::interp::dsl::record_call_exit!($tail, $mode, $name, st, t, s)
                        }
                    }
                } else {
                    match $crate::interp::ops::$name::fast(&mut st, $($oname),*) {
                        Done::Next => st.next,
                        Done::Jump(t) => t,
                    }
                });
                let St { cx, fp, acc, .. } = st;
                let to = $crate::interp::dsl::dispatch_pc_of!($name, next);
                // SAFETY: `to` is an instruction start of verified code or a VM stub.
                let h = cx.table[unsafe { *to } as usize];
                // SAFETY: the table holds handlers of this expansion; the state is valid.
                unsafe { $tail h(cx, to, fp, acc) }
            }
        } )*

        /// Cold handlers: one per opcode with a cold block (runtime calls live here).
        pub(crate) mod cold {
            #[allow(unused_imports)]
            use super::*;
            $(
                $crate::interp::ops::if_cold!($name {
                    $crate::interp::dsl::cold_handler!(
                        [$tail, $exp, $mode] $name ($($oname: $okind),*)
                    );
                } else {});
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
                if unsafe { $crate::interp::construct_throws(fp, acc) } {
                    // SAFETY: the same handler contract.
                    unsafe { $tail construct_return_throw(cx, pc, fp, acc) }
                }
                // SAFETY: caller contract; `construct_throws` is false, so no throw.
                let (next, fp, acc) = unsafe { $crate::interp::construct_result(fp, acc) };
                // SAFETY: `next` is the caller's next instruction.
                let h = cx.table[unsafe { *next } as usize];
                // SAFETY: as in the opcode handlers.
                unsafe { $tail h(cx, next, fp, acc) }
            }
        }

        $crate::interp::dsl::handler! {
            /// The cold block of `ConstructReturn` (H15 "Cold blocks"): the throw of a derived
            /// constructor's result that is not an object. It has the handler signature but no
            /// `ep9h__` symbol and is not in the table; `cargo xtask disasm-check` follows the
            /// tail jump to it as part of the handler. Not `#[cold]`: on x86-64 ELF that puts it
            /// in `.text.unlikely`, where `disasm-check` does not follow the jump.
            ///
            /// # Safety
            /// Only `construct_return` calls this, with its arguments.
            #[inline(never)]
            #[allow(unused_variables)]
            fn construct_return_throw(cx, pc, fp, acc) {
                // SAFETY: caller contract.
                let (next, fp, acc) = unsafe { $crate::interp::construct_return(cx, fp, acc) };
                // SAFETY: `next` is the catching handler's pc or the `Exit` stub.
                let h = cx.table[unsafe { *next } as usize];
                // SAFETY: as in the opcode handlers.
                unsafe { $tail h(cx, next, fp, acc) }
            }
        }

        $crate::interp::dsl::handler! {
            /// Spike B placement (b) (feature `spike-native`, P4-PLACEMENT-B): the stub byte
            /// `OP_ENTER_B`. `fp` is the frame of a function with a (b) slot that a call just
            /// pushed; this tail-enters its (b) code with the 4 handler arguments.
            ///
            /// # Safety
            /// Only the dispatch table calls this, with `fp` the new frame of a (b) callee.
            #[cfg(feature = "spike-native")]
            #[unsafe(export_name = concat!("ep9h__", $exp, "__tail__EnterB"))]
            #[allow(unused_variables)]
            fn enter_b(cx, pc, fp, acc) {
                #[cfg(debug_assertions)]
                $crate::interp::dsl::debug_native_stack_check(cx);
                // SAFETY: caller contract.
                let (h, code) = unsafe { $crate::native::b_target(fp) };
                // SAFETY: the (b) code has the handler ABI (`install_native_b` contract).
                unsafe { $tail h(cx, code, fp, acc) }
            }
        }

        $crate::interp::dsl::handler! {
            /// Spike B report M2-P4-B-AB (feature `spike-native`): the stub byte
            /// `native::OP_ENTER_A_HOT`. `fp` is the frame of a placement (a) callee (an
            /// AotBaseline slot) that `call_fast` just pushed; this runs its native code
            /// (`native::a_hot_call`) and continues at the caller's next instruction.
            ///
            /// # Safety
            /// Only the dispatch table calls this, with `fp` the new frame of such a callee.
            #[cfg(feature = "spike-native")]
            #[unsafe(export_name = concat!("ep9h__", $exp, "__tail__EnterAHot"))]
            #[allow(unused_variables)]
            fn enter_a_hot(cx, pc, fp, acc) {
                #[cfg(debug_assertions)]
                $crate::interp::dsl::debug_native_stack_check(cx);
                // SAFETY: caller contract.
                let acc = unsafe { $crate::native::a_hot_call(cx, fp) };
                let (next, fp) = (cx.out_pc, cx.out_fp);
                // SAFETY: `next` is the caller's next instruction or a handler's pc.
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
            #[cfg(feature = "spike-native")]
            {
                t[$crate::interp::OP_ENTER_B as usize] = enter_b as Handler;
                t[$crate::native::OP_ENTER_A_HOT as usize] = enter_a_hot as Handler;
            }
            t
        };
    };
}
pub(crate) use expand_threaded;

/// A fast body's flow after its record, for the expansions that record inline (the match
/// loop (a) and the prefixed step): `records` (the isolate's feedback mode is `full`,
/// `profile::records`) runs the record (`Flow::recorded`); else (`ics_only`, MODES-D11) the
/// record is dropped: `Record` and `RecordPair` give `Next`, `JumpRecordCall` gives its `Jump`.
/// Other flows pass through. Out of line in debug builds, as `Flow::recorded`.
#[cfg_attr(debug_assertions, inline(never))]
#[cfg_attr(not(debug_assertions), inline(always))]
pub(crate) fn settle(
    f: crate::interp::Flow,
    st: &crate::interp::St<'_>,
    records: bool,
) -> crate::interp::Flow {
    use crate::interp::Flow;
    if records {
        return f.recorded(st);
    }
    match f {
        Flow::Record(_) | Flow::RecordPair(_) => Flow::Next,
        Flow::JumpRecordCall(t, _) => Flow::Jump(t),
        f => f,
    }
}

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
            // The type-feedback mode (MODES-D11): one read per entry; the isolate's mode never
            // changes. The `ics_only` loop drops the records of the fast bodies (`settle`).
            let records = $crate::profile::records(cx);
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
                            let flow = $crate::interp::ops::$name::fast(&mut st, $($oname),*);
                            match $crate::interp::dsl::settle(flow, &st, records) {
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
                                // `settle` (above) turned these into `Next` or `Jump`.
                                Flow::Record(_) | Flow::RecordPair(_) | Flow::JumpRecordCall(..) => {
                                    unreachable!()
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
                        let (next, f, a) = unsafe { $crate::interp::construct_return(&mut *cx, fp, acc) };
                        pc = next;
                        fp = f;
                        acc = a;
                    }
                    // Spike B (feature `spike-native`, D4): the match loop never enters native
                    // code. A (b) or A-hot callee whose frame a call just pushed runs its
                    // bytecode instead (the fallback).
                    #[cfg(feature = "spike-native")]
                    $crate::interp::OP_ENTER_B | $crate::native::OP_ENTER_A_HOT => {
                        pc = $crate::native::bytecode_of(fp);
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
            #[cfg(test)]
            $crate::interp::dsl::count_scaled(0);
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
                            // Out of line in debug builds, as in the match loop: inline, the
                            // record locals of every arm made this frame ~3x bigger. The
                            // isolate's feedback mode is read here, on this rare path (prefixed
                            // instructions), so every table shares this step.
                            f @ (Flow::Record(_)
                            | Flow::RecordPair(_)
                            | Flow::JumpRecordCall(..)) => {
                                let records = $crate::profile::records(st.cx);
                                match $crate::interp::dsl::settle(f, &st, records) {
                                    Flow::Jump(t) => Done::Jump(t),
                                    _ => Done::Next,
                                }
                            }
                            Flow::Cold => {
                                let St { cx, fp, acc, .. } = st;
                                // SAFETY: the cold step takes the same state as this arm.
                                return unsafe { step_cold::<S>(cx, pc, fp, acc) };
                            }
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

        /// The cold blocks of `step::<S>`, out of line, so that the rare cold paths (runtime
        /// calls, lookups, IC misses) do not shape the register use of the fast arms. Same
        /// contract as `step::<S>`; called only after the opcode's fast body returned
        /// `Flow::Cold`: here, or in the `Wide`/`ExtraWide` fast body for the opcodes it runs
        /// itself (`St::scaled_cold`, M7-P4 TSPASS2).
        ///
        /// # Safety
        /// As for `step::<S>`.
        #[cold]
        #[inline(never)]
        #[allow(unused_mut, unused_variables, unused_unsafe, unreachable_code)]
        pub(crate) unsafe fn step_cold<S: $crate::interp::dsl::Scale>(
            cx: &mut Cx,
            pc: *const u8,
            fp: *mut u64,
            acc: u64,
        ) -> u64 {
            #[cfg(test)]
            $crate::interp::dsl::count_scaled(1);
            // SAFETY: a verified prefix is followed by an opcode byte.
            let op = unsafe { *pc.add(1) };
            match op {
                $( $code if const { ep9_bytecode::Opcode::$name.info().has_scalable_operands() } => {
                    $crate::interp::ops::if_cold!($name {{
                        // SAFETY: as in `step::<S>`.
                        let (len, ($($oname,)*)) = unsafe {
                            $crate::interp::dsl::decode!(S, pc; $($oname: $okind),*)
                        };
                        // SAFETY: as in `step::<S>`.
                        let mut st = unsafe { St::new(&mut *cx, pc, fp, acc, pc.add(len)) };
                        let next = match $crate::interp::ops::$name::cold(&mut st, $($oname),*) {
                            Done::Next => st.next,
                            Done::Jump(t) => t,
                        };
                        let (f, a) = (st.fp, st.acc);
                        cx.out_fp = f;
                        cx.out_pc = next;
                        a
                    }} else {{
                        let r = $crate::interp::slow::bad_opcode(cx, fp, pc);
                        cx.out_fp = fp;
                        cx.out_pc = $crate::interp::EXIT_STUB.as_ptr();
                        r
                    }})
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
thread_local! {
    /// Tests (M7-P4 TSPASS2): the runs of `interp::scaled::step` (index 0, a whole prefixed
    /// instruction) and of `interp::scaled::step_cold` (index 1, only its cold body) on this
    /// thread.
    pub(crate) static SCALED_RUNS: core::cell::Cell<[u64; 2]> = const { core::cell::Cell::new([0; 2]) };
}

/// Counts one run of `step` (0) or `step_cold` (1) in [`SCALED_RUNS`] (tests).
#[cfg(test)]
pub(crate) fn count_scaled(i: usize) {
    SCALED_RUNS.with(|c| {
        let mut n = c.get();
        n[i] += 1;
        c.set(n);
    });
}

// ---- the x86-64 register operand decode ------------------------------------------------------

impl R {
    /// [`R::from_operand`] for the register operand decoders of the handlers (`kind::Reg` and
    /// the other register kinds). x86-64: the same map as a select, which LLVM keeps as `lea`,
    /// `test`, `cmov` and puts the slot offset into the load or store address (3 instructions;
    /// the shift form of `from_operand` gave 4-5 there: `xor`, `test`, `setns`, `shl`, `add`
    /// or `mov`, `sar`, `and`, `add`). Other targets: `from_operand` (their code does not
    /// change).
    ///
    /// aarch64 (M7-P5 Q6 DISPATCH): a branch on the sign (`R::from_operand_branch`): `tbnz`,
    /// and `sub` on the negative arm only (the receiver and the parameters). A local (the
    /// usual operand) costs 1 instruction before the load or store, not 2 (M7-P4 TSPASS2:
    /// `lsr` and `sub` with a shifted register); a negative operand costs 2, as before (LLVM
    /// copies the rest of the handler into the negative arm). Star and Ldar: 6 instructions on
    /// the local path, not 7. Without the empty `asm!` block in the negative arm LLVM turns
    /// the branch into `sub`, `tst`, `csel` (3). Other targets: `from_operand`.
    #[inline(always)]
    pub fn of_operand(v: i32) -> R {
        #[cfg(target_arch = "x86_64")]
        {
            R::from_operand_select(v)
        }
        #[cfg(target_arch = "aarch64")]
        {
            R::from_operand_branch(v)
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            R::from_operand(v)
        }
    }

    /// The aarch64 form of [`R::of_operand`]: `v - HEADER` for a negative `v`, else `v`, with
    /// the subtraction on its own (cold) arm. It builds on every target for the test; only
    /// aarch64 has the `asm!` block, which keeps the arm a branch (LLVM does not speculate an
    /// `asm!` block into a select).
    #[cfg(any(target_arch = "aarch64", test))]
    #[inline(always)]
    fn from_operand_branch(v: i32) -> R {
        let o = v as isize;
        if o < 0 {
            core::hint::cold_path();
            #[allow(unused_mut)]
            let mut n = o;
            #[cfg(target_arch = "aarch64")]
            // SAFETY: an empty block (a comment): no instruction, no memory access, no flag
            // change. `n` comes back as it went in.
            unsafe {
                core::arch::asm!(
                    "/* {n} */",
                    n = inout(reg) n,
                    options(pure, nomem, nostack, preserves_flags)
                );
            }
            R(n - HEADER as isize)
        } else {
            R(o)
        }
    }

    /// The x86-64 form of [`R::of_operand`]. It builds on every target for the test.
    #[cfg(any(target_arch = "x86_64", test))]
    #[inline(always)]
    fn from_operand_select(v: i32) -> R {
        let o = v as isize;
        R(core::hint::select_unpredictable(
            o < 0,
            o - HEADER as isize,
            o,
        ))
    }
}

/// The `pc` that a threaded handler passes on, as the handler computed it. x86-64: through an
/// empty `asm!` block, so LLVM keeps the new `pc` in one register and reads the next opcode
/// at `[pc]`. Without it LLVM read the opcode at `[old pc + len]` and kept the old and the
/// new `pc` live, so it gave `lea` and `mov rsi` (2 instructions) where `add rsi` (1) does
/// (`Jump`, the `Test*` compares, `JumpIfJSReceiver`, `Wide`, `ExtraWide`). Other targets:
/// the same value (their code does not change). The handler arguments, the table read and
/// the tail call are as before.
#[inline(always)]
pub(crate) fn dispatch_pc(next: *const u8) -> *const u8 {
    #[cfg(target_arch = "x86_64")]
    {
        let mut a = next.expose_provenance();
        // SAFETY: an empty block (a comment): no instruction, no memory access, no flag
        // change. `a` comes back as it went in.
        unsafe {
            core::arch::asm!(
                "/* {a} */",
                a = inout(reg) a,
                options(pure, nomem, nostack, preserves_flags)
            );
        }
        core::ptr::with_exposed_provenance(a)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        next
    }
}

/// [`dispatch_pc`] of the threaded handler `$name`. `GetNamedProperty` and `TestTypeOf` take
/// `next` as it is: with the `asm!` block LLVM merged their dispatch tails (3 into 1), so the
/// hit paths of their jump tables ran one more `jmp` (x86-64 asm, X86SHAPE try 3).
macro_rules! dispatch_pc_of {
    (GetNamedProperty, $next:expr) => {
        $next
    };
    (TestTypeOf, $next:expr) => {
        $next
    };
    ($other:ident, $next:expr) => {
        $crate::interp::dsl::dispatch_pc($next)
    };
}
pub(crate) use dispatch_pc_of;

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

    /// The decoder map of every target, the x86-64 select form and the aarch64 branch form are
    /// `from_operand`, for every 1-byte and 2-byte operand and the 4-byte ends.
    #[test]
    fn register_operand_decoder_is_from_operand() {
        let ends = [
            i32::MIN,
            i32::MIN + 1,
            -65_536,
            65_535,
            i32::MAX - 1,
            i32::MAX,
        ];
        for v in (i32::from(i16::MIN)..=i32::from(i16::MAX)).chain(ends) {
            assert_eq!(R::of_operand(v), R::from_operand(v), "operand {v}");
            assert_eq!(R::from_operand_select(v), R::from_operand(v), "operand {v}");
            assert_eq!(R::from_operand_branch(v), R::from_operand(v), "operand {v}");
        }
        // The receiver operand (-1) at the 3 scales: fp - 9 (H14).
        let b = [0xffu8, 0xff, 0xff, 0xff];
        // SAFETY: `b` has 4 readable bytes.
        unsafe {
            assert_eq!(<kind::Reg as Operand>::read::<S1>(b.as_ptr()).0, -9);
            assert_eq!(<kind::RegOut as Operand>::read::<S2>(b.as_ptr()).0, -9);
            assert_eq!(<kind::RegPair as Operand>::read::<S4>(b.as_ptr()).0, -9);
        }
    }

    /// `dispatch_pc` gives back the pointer it gets (on x86-64 through its `asm!` block).
    #[test]
    fn dispatch_pc_is_the_identity() {
        let code = [0u8; 16];
        for i in 0..code.len() {
            let p = code.as_ptr().wrapping_add(i);
            assert_eq!(dispatch_pc(p), p);
        }
        assert!(dispatch_pc(core::ptr::null()).is_null());
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
