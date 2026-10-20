//! The opcode DSL (H15). `ops.rs` lists every opcode once, in `for_each_op!`. The macros here
//! expand that one list in different ways:
//!
//! | Macro | Output |
//! |---|---|
//! | `expand_meta!` | `Op` enum and the `OPS` table, for the assembler and the verifier |
//! | `expand_match!` | (a) one `loop { match }` function |
//! | `expand_threaded!` + `return` | (b) one handler per opcode; each ends in `return table[op](..)` (sibling call) |
//! | `expand_threaded!` + `become` | (c) the same with nightly `become` (guaranteed tail call) |
//!
//! Handler symbol contract (read by `cargo xtask disasm-check`):
//! `ep9h__<expansion>__<class>__<op>`, where class is `tail` (must leave only through a tail
//! jump) or `exit` (may return to the host).

/// What a handler body does after its work.
pub enum Flow {
    /// Go to the next instruction.
    Next,
    /// Go to this pc.
    Jump(*const u8),
    /// Leave the interpreter with this value. Only `exit` opcodes use it.
    Exit(i64),
    /// Continue in this opcode's cold block (a separate cold handler in (b) and (c)).
    Cold,
}

/// Operand kinds. Each kind has a fixed size in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperandKind {
    /// Register index (u8). The verifier checks it against the frame size.
    Reg,
    /// Signed 8-bit immediate.
    Imm,
    /// Constant-pool index (u8).
    Const,
    /// Function index (u8).
    Func,
    /// Argument count (u8).
    Cnt,
    /// Jump offset (i16, little-endian), relative to the start of the instruction.
    Off,
}

impl OperandKind {
    pub const fn size(self) -> usize {
        match self {
            OperandKind::Off => 2,
            _ => 1,
        }
    }
}

/// Decoder for one operand kind.
pub trait Operand {
    const KIND: OperandKind;
    const SIZE: usize = Self::KIND.size();
    type Value;
    /// # Safety
    /// `p` must point at `SIZE` readable bytes.
    unsafe fn read(p: *const u8) -> Self::Value;
}

macro_rules! byte_operand {
    ($name:ident, $kind:ident, $ty:ty, |$b:ident| $conv:expr) => {
        pub struct $name;
        impl Operand for $name {
            const KIND: OperandKind = OperandKind::$kind;
            type Value = $ty;
            #[inline(always)]
            unsafe fn read(p: *const u8) -> $ty {
                // SAFETY: the caller promises one readable byte at `p`.
                let $b = unsafe { *p };
                $conv
            }
        }
    };
}

byte_operand!(Reg, Reg, usize, |b| b as usize);
byte_operand!(Imm, Imm, i64, |b| b as i8 as i64);
byte_operand!(Const, Const, usize, |b| b as usize);
byte_operand!(Func, Func, usize, |b| b as usize);
byte_operand!(Cnt, Cnt, usize, |b| b as usize);

pub struct Off;
impl Operand for Off {
    const KIND: OperandKind = OperandKind::Off;
    type Value = isize;
    #[inline(always)]
    unsafe fn read(p: *const u8) -> isize {
        // SAFETY: the caller promises two readable bytes at `p`. read_unaligned has no
        // alignment need.
        let v = unsafe { p.cast::<i16>().read_unaligned() };
        i16::from_le(v) as isize
    }
}

// ---- Handler signature rule (H15): at most 4 arguments, each one register wide. ----

/// A handler argument that fits in one integer register.
pub trait RegArg {}
impl RegArg for i64 {}
impl RegArg for u64 {}
impl RegArg for usize {}
impl<T> RegArg for *const T {}
impl<T> RegArg for *mut T {}

/// Implemented only for handler types with at most 4 register-wide arguments. The first
/// argument is the context reference. Win64 passes only 4 integer arguments in registers;
/// a 5th goes on the stack and can break the sibling call (RK25).
pub trait AtMostFourRegArgs {}
impl<C, R: RegArg> AtMostFourRegArgs for for<'a> unsafe fn(&'a mut C) -> R {}
impl<C, A: RegArg, R: RegArg> AtMostFourRegArgs for for<'a> unsafe fn(&'a mut C, A) -> R {}
impl<C, A: RegArg, B: RegArg, R: RegArg> AtMostFourRegArgs
    for for<'a> unsafe fn(&'a mut C, A, B) -> R
{
}
impl<C, A: RegArg, B: RegArg, D: RegArg, R: RegArg> AtMostFourRegArgs
    for for<'a> unsafe fn(&'a mut C, A, B, D) -> R
{
}

/// Compile-time check. A handler type with 5 arguments, or with an argument that is not
/// register wide, fails here.
pub const fn assert_handler_sig<T: AtMostFourRegArgs>() {}

/// Calls a handler. With `neg-five-args`, every call passes a 5th argument that changes on
/// each dispatch, so it cannot stay in the caller's incoming stack slot.
#[cfg(not(feature = "neg-five-args"))]
macro_rules! call_handler {
    ($h:expr, $cx:expr, $pc:expr, $fp:expr, $acc:expr, $extra:expr) => {
        $h($cx, $pc, $fp, $acc)
    };
}
#[cfg(feature = "neg-five-args")]
macro_rules! call_handler {
    ($h:expr, $cx:expr, $pc:expr, $fp:expr, $acc:expr, $extra:expr) => {
        $h($cx, $pc, $fp, $acc, $extra)
    };
}
pub(crate) use call_handler;

macro_rules! kind_is_exit {
    (exit) => {
        true
    };
    (tail) => {
        false
    };
}
pub(crate) use kind_is_exit;

/// Op enum, metadata table and opcode checks.
macro_rules! expand_meta {
    ( []
      state($cx:ident, $pc:ident, $npc:ident, $fp:ident, $acc:ident);
      $( $(#[$doc:meta])* $kind:ident $snake:ident $Name:ident = $code:literal
         ( $( $opnd:ident : $oty:ident ),* ) => $body:block $(cold $cold:block)? ; )*
    ) => {
        /// Opcodes. The byte value is the opcode.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u8)]
        pub enum Op { $( $(#[$doc])* $Name = $code, )* }

        /// Static facts about one opcode.
        #[derive(Debug)]
        pub struct OpInfo {
            pub op: Op,
            pub name: &'static str,
            /// True if the handler may return to the host (class `exit`).
            pub exit: bool,
            /// True if the opcode has a cold block (a separate cold handler).
            pub cold: bool,
            pub operands: &'static [$crate::dsl::OperandKind],
        }

        impl OpInfo {
            /// Instruction length in bytes: opcode plus operands.
            pub const fn insn_len(&self) -> usize {
                let mut n = 1;
                let mut i = 0;
                while i < self.operands.len() {
                    n += self.operands[i].size();
                    i += 1;
                }
                n
            }
        }

        /// One entry per opcode, in opcode order.
        pub const OPS: &[OpInfo] = &[ $( OpInfo {
            op: Op::$Name,
            name: stringify!($snake),
            exit: $crate::dsl::kind_is_exit!($kind),
            cold: false $( || { let _ = stringify!($cold); true } )?,
            operands: &[ $( <$crate::dsl::$oty as $crate::dsl::Operand>::KIND ),* ],
        }, )* ];

        impl Op {
            pub const fn from_u8(b: u8) -> Option<Op> {
                match b { $( $code => Some(Op::$Name), )* _ => None }
            }
            pub const fn info(self) -> &'static OpInfo {
                &OPS[self as usize]
            }
        }

        // Opcodes are dense and in order, so OPS[op as usize] is the entry for op.
        const _: () = {
            let codes: &[u8] = &[ $( $code ),* ];
            let mut i = 0;
            while i < codes.len() {
                assert!(codes[i] as usize == i, "opcodes must be 0, 1, 2, ... in list order");
                i += 1;
            }
        };
    };
}
pub(crate) use expand_meta;

/// Called when a body returns `Flow::Cold` but its opcode has no cold block. The DSL never
/// does this; the call is dead code that LLVM removes.
#[cold]
#[inline(never)]
pub fn no_cold_block() -> ! {
    panic!("Flow::Cold from an opcode without a cold block")
}

/// (a) The match loop. With `[count]`, it also counts dispatches in `Cx::dispatches`.
/// A cold block runs inline in its match arm.
macro_rules! expand_match {
    ( [$fname:ident $(, $count:ident)?]
      state($cx:ident, $pc:ident, $npc:ident, $fp:ident, $acc:ident);
      $( $(#[$doc:meta])* $kind:ident $snake:ident $Name:ident = $code:literal
         ( $( $opnd:ident : $oty:ident ),* ) => $body:block $(cold $cold:block)? ; )*
    ) => {
        /// Runs from `pc` until an `exit` opcode.
        ///
        /// # Safety
        /// `pc` and `fp` must come from `vm::run` on a verified program.
        #[inline(never)]
        #[allow(unused_unsafe, unused_mut, unused_variables, unused_assignments, unreachable_code)]
        pub unsafe fn $fname(cx: &mut $crate::vm::Cx, pc: *const u8, fp: *mut i64, acc: i64) -> i64 {
            let $cx = cx;
            let mut $pc = pc;
            let mut $fp = fp;
            let mut $acc = acc;
            loop {
                $( $crate::dsl::expand_match!(@count $count, $cx); )?
                // SAFETY: `pc` points at an opcode byte of verified code.
                let op = unsafe { *$pc };
                match op {
                    $( $code => {
                        const LEN: usize = 1 $( + <$crate::dsl::$oty as $crate::dsl::Operand>::SIZE )*;
                        // SAFETY: the verifier checked every operand of this instruction:
                        // registers are inside the frame, indexes inside their tables, jump
                        // targets on instruction starts. `vm::run` set up `fp` and `cx`.
                        let flow = unsafe {
                            let $npc = $pc.add(LEN);
                            let mut at = 1usize;
                            $(
                                let $opnd = <$crate::dsl::$oty as $crate::dsl::Operand>::read($pc.add(at));
                                at += <$crate::dsl::$oty as $crate::dsl::Operand>::SIZE;
                            )*
                            match $body {
                                $crate::dsl::Flow::Cold => $crate::dsl::expand_match!(@cold $($cold)?),
                                f => f,
                            }
                        };
                        match flow {
                            // SAFETY: a verified function never ends with a fall-through
                            // opcode, so pc + LEN is the next instruction.
                            $crate::dsl::Flow::Next => $pc = unsafe { $pc.add(LEN) },
                            $crate::dsl::Flow::Jump(t) => $pc = t,
                            $crate::dsl::Flow::Exit(v) => return v,
                            $crate::dsl::Flow::Cold => $crate::dsl::no_cold_block(),
                        }
                    } )*
                    // SAFETY: the verifier accepts only defined opcodes.
                    _ => unsafe { core::hint::unreachable_unchecked() },
                }
            }
        }
    };
    (@count count, $cx:ident) => { $cx.dispatches += 1; };
    (@cold) => { $crate::dsl::no_cold_block() };
    (@cold $cold:block) => { $cold };
}
pub(crate) use expand_match;

/// (b) and (c): one handler per opcode, plus one cold handler per cold block. `$tail` is
/// `return` (sibling call; LLVM may or may not make it a tail jump, so
/// `cargo xtask disasm-check` verifies it) or `become` (nightly, guaranteed). `$exp` names
/// the expansion in the handler symbols.
///
/// A fast handler reaches its cold handler by a tail jump, so the fast path makes no calls
/// and needs no stack frame. The cold handler decodes the operands again.
macro_rules! expand_threaded {
    ( [$tail:tt, $exp:literal]
      state($cx:ident, $pc:ident, $npc:ident, $fp:ident, $acc:ident);
      $( $(#[$doc:meta])* $kind:ident $snake:ident $Name:ident = $code:literal
         ( $( $opnd:ident : $oty:ident ),* ) => $body:block $(cold $cold:block)? ; )*
    ) => {
        $(
            $crate::dsl::expand_threaded!(@handler [$tail, $exp, $kind]
                $(#[$doc])*
                #[unsafe(export_name = concat!("ep9h__", $exp, "__", stringify!($kind), "__", stringify!($snake)))]
                fn $snake ($cx, $pc, $npc, $fp, $acc) ($( $opnd : $oty ),*) $body
                inline_cold { $($cold)? }
                cold_arm { $crate::dsl::expand_threaded!(@cold_arm $tail, $snake, $cx, $pc, $fp, $acc; $($cold)?) }
            );
        )*

        /// Cold handlers: one per opcode with a cold block.
        pub mod cold {
            #[allow(unused_imports)]
            use $crate::dsl::Flow;
            $(
                $crate::dsl::expand_threaded!(@cold_handler [$tail, $exp]
                    $snake ($cx, $pc, $npc, $fp, $acc) ($( $opnd : $oty ),*); $($cold)?
                );
            )*
        }

        /// Fills every unused table slot. The verifier rejects unknown opcodes, so this
        /// runs only on a verifier bug.
        ///
        /// # Safety
        /// Only the dispatch table calls this.
        #[unsafe(export_name = concat!("ep9h__", $exp, "__exit__illegal"))]
        #[allow(unused_variables)]
        unsafe fn illegal(
            cx: &mut $crate::vm::Cx,
            pc: *const u8,
            fp: *mut i64,
            acc: i64,
            #[cfg(feature = "neg-five-args")] extra: u64,
        ) -> i64 {
            cx.error = $crate::vm::ERR_ILLEGAL;
            i64::MIN
        }

        /// The dispatch table for this expansion. `vm::run` copies it into `Cx::table`.
        pub static TABLE: [$crate::vm::Handler; 256] = {
            let mut t = [illegal as $crate::vm::Handler; 256];
            $( t[$code] = $snake; )*
            t
        };
    };

    // One handler function: decode, run the body, then continue.
    (@handler [$tail:tt, $exp:literal, $kind:ident]
      $(#[$attr:meta])*
      $vis:vis fn $fname:ident ($cx:ident, $pc:ident, $npc:ident, $fp:ident, $acc:ident)
      ($( $opnd:ident : $oty:ident ),*) $body:block
      inline_cold { $($inline_cold:block)? }
      cold_arm $cold_arm:block
    ) => {
        $(#[$attr])*
        ///
        /// # Safety
        /// Only the dispatch table (or the fast handler) calls this, with state from `vm::run`.
        #[allow(unused_unsafe, unused_mut, unused_variables, unused_assignments, unreachable_code)]
        $vis unsafe fn $fname(
            $cx: &mut $crate::vm::Cx,
            $pc: *const u8,
            mut $fp: *mut i64,
            mut $acc: i64,
            #[cfg(feature = "neg-five-args")] extra: u64,
        ) -> i64 {
            const LEN: usize = 1 $( + <$crate::dsl::$oty as $crate::dsl::Operand>::SIZE )*;
            // SAFETY: as in the match loop: verified operands, state from `vm::run`.
            let flow = unsafe {
                let $npc = $pc.add(LEN);
                let mut at = 1usize;
                $(
                    let $opnd = <$crate::dsl::$oty as $crate::dsl::Operand>::read($pc.add(at));
                    at += <$crate::dsl::$oty as $crate::dsl::Operand>::SIZE;
                )*
                let f = $body;
                // A/B switch for the README: run the cold block inside the fast handler
                // (runtime calls on the fast path), as a plain design would.
                #[cfg(feature = "cold-inline")]
                let f = match f {
                    $( $crate::dsl::Flow::Cold => $inline_cold, )?
                    f => f,
                };
                f
            };
            let next = match flow {
                // SAFETY: a verified function never ends with a fall-through opcode.
                $crate::dsl::Flow::Next => unsafe { $pc.add(LEN) },
                $crate::dsl::Flow::Jump(t) => t,
                $crate::dsl::Flow::Exit(v) => return v,
                $crate::dsl::Flow::Cold => $cold_arm,
            };
            // SAFETY: `next` points at an opcode byte of verified code.
            let op = unsafe { *next };
            let h = $cx.table[op as usize];
            // SAFETY: the table holds only handlers of this expansion, and they all take
            // the state that `vm::run` set up.
            unsafe {
                $tail $crate::dsl::call_handler!(h, $cx, next, $fp, $acc, extra.wrapping_add(1))
            }
        }
    };

    // A cold handler for an opcode with a cold block; nothing for the others.
    (@cold_handler [$tail:tt, $exp:literal] $snake:ident $state:tt $opnds:tt; ) => {};
    (@cold_handler [$tail:tt, $exp:literal] $snake:ident $state:tt $opnds:tt; $cold:block) => {
        $crate::dsl::expand_threaded!(@handler [$tail, $exp, tail]
            /// Cold path of the opcode with the same name.
            #[unsafe(export_name = concat!("ep9h__", $exp, "__tail__", stringify!($snake), "_cold"))]
            #[cold]
            #[inline(never)]
            pub(super) fn $snake $state $opnds $cold
            inline_cold {}
            cold_arm { $crate::dsl::no_cold_block() }
        );
    };

    // Flow::Cold in a fast handler: tail jump to the cold handler, if there is one.
    (@cold_arm $tail:tt, $snake:ident, $cx:ident, $pc:ident, $fp:ident, $acc:ident; ) => {
        $crate::dsl::no_cold_block()
    };
    (@cold_arm $tail:tt, $snake:ident, $cx:ident, $pc:ident, $fp:ident, $acc:ident; $cold:block) => {
        // SAFETY: the cold handler takes the same state as this handler.
        unsafe {
            $tail $crate::dsl::call_handler!(cold::$snake, $cx, $pc, $fp, $acc, 0)
        }
    };
}
pub(crate) use expand_threaded;

const _: () = {
    #[cfg(not(feature = "neg-skip-arg-check"))]
    assert_handler_sig::<crate::vm::Handler>();
};
