//! Win64 unwind data for x64 code (Microsoft "x64 exception handling").
//!
//! ```text
//!   RUNTIME_FUNCTION  begin:u32 | end:u32 | unwind_info:u32     RVAs from the table's range base
//!   UNWIND_INFO       ver=1|flags<<3 | prolog size | code count | frame reg | frame off<<4
//!                     codes, last prolog instruction first; padded to an even count
//!   UNWIND_CODE       prolog offset (end of the instruction) | op | info<<4 [| extra slots]
//! ```
//!
//! The epilogue needs no data: the unwinder recognizes `add rsp, n` / `lea rsp, ..`, pops and
//! `ret` by their bytes.

/// Register numbers in unwind codes and in the frame-register field.
pub mod reg {
    pub const RAX: u8 = 0;
    pub const RCX: u8 = 1;
    pub const RDX: u8 = 2;
    pub const RBX: u8 = 3;
    pub const RSP: u8 = 4;
    pub const RBP: u8 = 5;
    pub const RSI: u8 = 6;
    pub const RDI: u8 = 7;
    pub const R12: u8 = 12;
    pub const R13: u8 = 13;
    pub const R14: u8 = 14;
    pub const R15: u8 = 15;
}

const UWOP_PUSH_NONVOL: u8 = 0;
const UWOP_ALLOC_LARGE: u8 = 1;
const UWOP_ALLOC_SMALL: u8 = 2;
const UWOP_SET_FPREG: u8 = 3;
const UWOP_SAVE_NONVOL: u8 = 4;

/// `IMAGE_RUNTIME_FUNCTION_ENTRY` (x64): one function. Addresses are offsets from the base of
/// the registered range.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RuntimeFunction {
    pub begin: u32,
    pub end: u32,
    /// Offset of the function's UNWIND_INFO; 4-byte aligned.
    pub unwind_info: u32,
}

/// Builds an UNWIND_INFO for a prolog. Call the methods in prolog order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnwindInfoBuilder {
    prolog_size: u8,
    frame_reg: u8,
    frame_offset: u8,
    /// One entry per prolog instruction: its slots (2 bytes each).
    ops: Vec<Vec<[u8; 2]>>,
}

impl UnwindInfoBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    fn op(&mut self, end: u8, op: u8, info: u8, extra: &[u16]) -> &mut Self {
        assert!(info < 16, "unwind op info is 4 bits");
        assert!(
            end >= self.prolog_size,
            "prolog instructions must be added in order"
        );
        let mut slots = vec![[end, op | (info << 4)]];
        slots.extend(extra.iter().map(|s| s.to_le_bytes()));
        self.ops.push(slots);
        self.prolog_size = end;
        self
    }

    /// `push reg`, which ends at prolog offset `end`.
    pub fn push_nonvol(&mut self, end: u8, reg: u8) -> &mut Self {
        self.op(end, UWOP_PUSH_NONVOL, reg, &[])
    }

    /// `sub rsp, size`. `size` is a non-zero multiple of 8.
    pub fn alloc(&mut self, end: u8, size: u32) -> &mut Self {
        assert!(
            size != 0 && size.is_multiple_of(8),
            "stack allocation is a non-zero multiple of 8"
        );
        if size <= 128 {
            self.op(end, UWOP_ALLOC_SMALL, (size / 8 - 1) as u8, &[])
        } else if size / 8 <= 0xffff {
            self.op(end, UWOP_ALLOC_LARGE, 0, &[(size / 8) as u16])
        } else {
            self.op(
                end,
                UWOP_ALLOC_LARGE,
                1,
                &[size as u16, (size >> 16) as u16],
            )
        }
    }

    /// `lea reg, [rsp + offset]` that makes `reg` the frame register. `offset` is a multiple of
    /// 16, at most 240.
    pub fn set_fpreg(&mut self, end: u8, reg: u8, offset: u8) -> &mut Self {
        assert!(
            offset.is_multiple_of(16) && offset <= 240,
            "frame offset is a multiple of 16, at most 240"
        );
        self.frame_reg = reg;
        self.frame_offset = offset / 16;
        self.op(end, UWOP_SET_FPREG, 0, &[])
    }

    /// `mov [rsp + offset], reg`. `offset` is a multiple of 8 below 512 KiB.
    pub fn save_nonvol(&mut self, end: u8, reg: u8, offset: u32) -> &mut Self {
        assert!(
            offset.is_multiple_of(8) && offset / 8 <= 0xffff,
            "save offset is a multiple of 8 below 512 KiB"
        );
        self.op(end, UWOP_SAVE_NONVOL, reg, &[(offset / 8) as u16])
    }

    /// The UNWIND_INFO bytes (version 1, no handler). Length is a multiple of 4.
    pub fn encode(&self) -> Vec<u8> {
        let slots: Vec<[u8; 2]> = self.ops.iter().rev().flatten().copied().collect();
        assert!(slots.len() <= 255, "too many unwind codes");
        let mut out = vec![
            1,
            self.prolog_size,
            slots.len() as u8,
            self.frame_reg | (self.frame_offset << 4),
        ];
        for s in &slots {
            out.extend_from_slice(s);
        }
        if slots.len() % 2 == 1 {
            out.extend_from_slice(&[0, 0]);
        }
        out
    }
}
