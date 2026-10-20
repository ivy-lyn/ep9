//! Hand-assembled generated code for the host, and its unwind data.
//!
//! ```text
//!   outer(x, cb, inner):  frame; x19/rbx := MARK; inner(x, cb); return its result
//!   inner(x, cb):         frame; x19/rbx := x;    r = cb(x + IMM); return r + x
//! ```
//!
//! Both functions save the callee-saved register they use. So a walk from `cb` must restore
//! x19/rbx = MARK in the outer frame: that checks the saved-register rules of the unwind data,
//! not only the return addresses.
//!
//! Bytes checked with `clang -c` + `objdump -d` (aarch64, x86-64) and `llvm-mc` +
//! `llvm-readobj --unwind` (Win64 UNWIND_INFO), 2026-10-21.

#[cfg(unix)]
use ep9_platform::unwind::cfi::{CfiProgram, Cie, EhFrameBuilder, EhFrameSection};

/// Value `outer` puts in x19 / rbx.
pub const MARK: u64 = 0xbeef;
pub const INNER_OFF: usize = 0;
pub const OUTER_OFF: usize = 128;

// ---------------------------------------------------------------------------------------------
#[cfg(target_arch = "aarch64")]
mod arch {
    fn words(w: &[u32]) -> Vec<u8> {
        w.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// Return address of `blr x1` in inner and `blr x2` in outer.
    pub const INNER_RET: usize = 24;
    pub const OUTER_RET: usize = 20;
    pub const IMM_MAX: u32 = 4095;
    /// The callee-saved register the check reads: x19 (DWARF 19).
    pub const SAVED_REG_DWARF: i32 = 19;

    pub fn inner() -> Vec<u8> {
        words(&[
            0xa9be_7bfd, //  0 stp x29, x30, [sp, #-32]!
            0x9100_03fd, //  4 mov x29, sp
            0xa901_53f3, //  8 stp x19, x20, [sp, #16]
            0xaa00_03f3, // 12 mov x19, x0
            add_imm(1),  // 16 add x0, x0, #imm
            0xd63f_0020, // 20 blr x1
            0x8b13_0000, // 24 add x0, x0, x19
            0xa941_53f3, // 28 ldp x19, x20, [sp, #16]
            0xa8c2_7bfd, // 32 ldp x29, x30, [sp], #32
            0xd65f_03c0, // 36 ret
        ])
    }

    pub fn outer() -> Vec<u8> {
        words(&[
            0xa9be_7bfd, //  0 stp x29, x30, [sp, #-32]!
            0x9100_03fd, //  4 mov x29, sp
            0xa901_53f3, //  8 stp x19, x20, [sp, #16]
            0xd297_ddf3, // 12 movz x19, #0xbeef
            0xd63f_0040, // 16 blr x2
            0xa941_53f3, // 20 ldp x19, x20, [sp, #16]
            0xa8c2_7bfd, // 24 ldp x29, x30, [sp], #32
            0xd65f_03c0, // 28 ret
        ])
    }

    fn add_imm(k: u32) -> u32 {
        0x9100_0000 | (k << 10)
    }

    /// The bytes to write at `inner + offset` for `add x0, x0, #k`.
    pub fn inner_imm_patch(k: u32) -> (usize, Vec<u8>) {
        (16, add_imm(k).to_le_bytes().to_vec())
    }

    #[cfg(unix)]
    pub fn inner_cfi() -> super::CfiProgram {
        use ep9_platform::unwind::cfi::reg::aarch64::*;
        let mut p = super::CfiProgram::new();
        p.advance_loc(1) // 4
            .def_cfa_offset(32)
            .offset(FP, 4)
            .offset(LR, 3)
            .advance_loc(1) // 8
            .def_cfa_register(FP)
            .advance_loc(1) // 12
            .offset(X19, 2)
            .offset(X20, 1)
            .advance_loc(5) // 32
            .restore(X19)
            .restore(X20)
            .advance_loc(1) // 36
            .def_cfa(SP, 0)
            .restore(FP)
            .restore(LR);
        p
    }

    #[cfg(unix)]
    pub fn outer_cfi() -> super::CfiProgram {
        use ep9_platform::unwind::cfi::reg::aarch64::*;
        let mut p = super::CfiProgram::new();
        p.advance_loc(1) // 4
            .def_cfa_offset(32)
            .offset(FP, 4)
            .offset(LR, 3)
            .advance_loc(1) // 8
            .def_cfa_register(FP)
            .advance_loc(1) // 12
            .offset(X19, 2)
            .offset(X20, 1)
            .advance_loc(3) // 24
            .restore(X19)
            .restore(X20)
            .advance_loc(1) // 28
            .def_cfa(SP, 0)
            .restore(FP)
            .restore(LR);
        p
    }
}

// ---------------------------------------------------------------------------------------------
#[cfg(all(target_arch = "x86_64", not(windows)))]
mod arch {
    pub const INNER_RET: usize = 18;
    pub const OUTER_RET: usize = 16;
    pub const IMM_MAX: u32 = 127;
    /// rbx (DWARF 3).
    pub const SAVED_REG_DWARF: i32 = 3;

    pub fn inner() -> Vec<u8> {
        #[rustfmt::skip]
        let v = vec![
            0x55,                   //  0 push rbp
            0x48, 0x89, 0xe5,       //  1 mov rbp, rsp
            0x53,                   //  4 push rbx
            0x48, 0x83, 0xec, 0x08, //  5 sub rsp, 8
            0x48, 0x89, 0xfb,       //  9 mov rbx, rdi
            0x48, 0x8d, 0x7f, 0x01, // 12 lea rdi, [rdi + imm8]
            0xff, 0xd6,             // 16 call rsi
            0x48, 0x01, 0xd8,       // 18 add rax, rbx
            0x48, 0x83, 0xc4, 0x08, // 21 add rsp, 8
            0x5b,                   // 25 pop rbx
            0x5d,                   // 26 pop rbp
            0xc3,                   // 27 ret
        ];
        v
    }

    pub fn outer() -> Vec<u8> {
        #[rustfmt::skip]
        let v = vec![
            0x55,                         //  0 push rbp
            0x48, 0x89, 0xe5,             //  1 mov rbp, rsp
            0x53,                         //  4 push rbx
            0x48, 0x83, 0xec, 0x08,       //  5 sub rsp, 8
            0xbb, 0xef, 0xbe, 0x00, 0x00, //  9 mov ebx, 0xbeef
            0xff, 0xd2,                   // 14 call rdx
            0x48, 0x83, 0xc4, 0x08,       // 16 add rsp, 8
            0x5b,                         // 20 pop rbx
            0x5d,                         // 21 pop rbp
            0xc3,                         // 22 ret
        ];
        v
    }

    pub fn inner_imm_patch(k: u32) -> (usize, Vec<u8>) {
        (15, vec![k as u8])
    }

    pub fn inner_cfi() -> super::CfiProgram {
        use ep9_platform::unwind::cfi::reg::x86_64::*;
        let mut p = super::CfiProgram::new();
        p.advance_loc(1) // 1
            .def_cfa_offset(16)
            .offset(RBP, 2)
            .advance_loc(3) // 4
            .def_cfa_register(RBP)
            .advance_loc(1) // 5
            .offset(RBX, 3)
            .advance_loc(21) // 26
            .restore(RBX)
            .advance_loc(1) // 27
            .def_cfa(RSP, 8)
            .restore(RBP);
        p
    }

    pub fn outer_cfi() -> super::CfiProgram {
        use ep9_platform::unwind::cfi::reg::x86_64::*;
        let mut p = super::CfiProgram::new();
        p.advance_loc(1) // 1
            .def_cfa_offset(16)
            .offset(RBP, 2)
            .advance_loc(3) // 4
            .def_cfa_register(RBP)
            .advance_loc(1) // 5
            .offset(RBX, 3)
            .advance_loc(16) // 21
            .restore(RBX)
            .advance_loc(1) // 22
            .def_cfa(RSP, 8)
            .restore(RBP);
        p
    }
}

// ---------------------------------------------------------------------------------------------
#[cfg(all(target_arch = "x86_64", windows))]
mod arch {
    use ep9_platform::unwind::win64::{UnwindInfoBuilder, reg};

    pub const INNER_RET: usize = 14;
    pub const OUTER_RET: usize = 13;
    pub const IMM_MAX: u32 = 127;

    pub fn inner() -> Vec<u8> {
        #[rustfmt::skip]
        let v = vec![
            0x53,                   //  0 push rbx
            0x48, 0x83, 0xec, 0x20, //  1 sub rsp, 32 (shadow space)
            0x48, 0x89, 0xcb,       //  5 mov rbx, rcx
            0x48, 0x8d, 0x49, 0x01, //  8 lea rcx, [rcx + imm8]
            0xff, 0xd2,             // 12 call rdx
            0x48, 0x01, 0xd8,       // 14 add rax, rbx
            0x48, 0x83, 0xc4, 0x20, // 17 add rsp, 32
            0x5b,                   // 21 pop rbx
            0xc3,                   // 22 ret
        ];
        v
    }

    pub fn outer() -> Vec<u8> {
        #[rustfmt::skip]
        let v = vec![
            0x53,                         //  0 push rbx
            0x48, 0x83, 0xec, 0x20,       //  1 sub rsp, 32
            0xbb, 0xef, 0xbe, 0x00, 0x00, //  5 mov ebx, 0xbeef
            0x41, 0xff, 0xd0,             // 10 call r8
            0x48, 0x83, 0xc4, 0x20,       // 13 add rsp, 32
            0x5b,                         // 17 pop rbx
            0xc3,                         // 18 ret
        ];
        v
    }

    pub fn inner_imm_patch(k: u32) -> (usize, Vec<u8>) {
        (11, vec![k as u8])
    }

    /// Both prologs: `push rbx` ends at 1, `sub rsp, 32` ends at 5.
    pub fn unwind_info() -> Vec<u8> {
        UnwindInfoBuilder::new()
            .push_nonvol(1, reg::RBX)
            .alloc(5, 32)
            .encode()
    }
}

pub use arch::*;

/// The `.eh_frame` section for both functions at their executable addresses.
#[cfg(unix)]
pub fn eh_frame(inner_addr: usize, outer_addr: usize) -> EhFrameSection {
    let mut b = EhFrameBuilder::new(&Cie::host());
    b.add_fde(inner_addr, inner().len(), &inner_cfi());
    b.add_fde(outer_addr, outer().len(), &outer_cfi());
    b.finish()
}
