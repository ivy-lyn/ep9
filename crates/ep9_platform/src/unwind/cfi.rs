//! DWARF call frame information in `.eh_frame` form, for the unix unwinders (libgcc on Linux,
//! libunwind on macOS).
//!
//! ```text
//!   .eh_frame section built here (little endian, every record 8-byte aligned):
//!   CIE  len:u32 | id=0:u32 | ver=1 | "zR\0" | code_align:uleb | data_align:sleb | ra:u8
//!        | aug_len=1 | fde_enc=absptr | initial instructions | nop padding
//!   FDE  len:u32 | cie_ptr:u32 (back to the CIE) | pc_begin:u64 | pc_range:u64 | aug_len=0
//!        | instructions | nop padding
//!   ...  more FDEs
//!   end  0:u32
//! ```
//!
//! - `pc_begin` is absolute (`DW_EH_PE_absptr`), so the section bytes do not depend on where
//!   the section itself lives.
//! - Only the instructions that generated code needs are here. `ep9_masm` will add more.

use std::io;

/// `DW_EH_PE_absptr`: a native-size absolute pointer.
const DW_EH_PE_ABSPTR: u8 = 0x00;

/// DWARF register numbers.
pub mod reg {
    /// AArch64 (DWARF for the Arm 64-bit architecture).
    pub mod aarch64 {
        pub const X19: u16 = 19;
        pub const X20: u16 = 20;
        pub const FP: u16 = 29;
        pub const LR: u16 = 30;
        pub const SP: u16 = 31;
    }
    /// x86-64 (System V psABI, figure 3.36).
    pub mod x86_64 {
        pub const RBX: u16 = 3;
        pub const RBP: u16 = 6;
        pub const RSP: u16 = 7;
        /// The return-address column.
        pub const RA: u16 = 16;
    }
}

fn uleb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn sleb(out: &mut Vec<u8>, mut v: i64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        let done = (v == 0 && b & 0x40 == 0) || (v == -1 && b & 0x40 != 0);
        if done {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// Call frame instructions (DWARF 5 section 6.4.2). Offsets of saved registers are factored:
/// the unwinder multiplies them by the CIE's data alignment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CfiProgram(Vec<u8>);

impl CfiProgram {
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the location forward by `delta` code-alignment units.
    pub fn advance_loc(&mut self, delta: u32) -> &mut Self {
        match delta {
            0 => {}
            1..=0x3f => self.0.push(0x40 | delta as u8),
            0x40..=0xff => self.0.extend_from_slice(&[0x02, delta as u8]),
            0x100..=0xffff => {
                self.0.push(0x03);
                self.0.extend_from_slice(&(delta as u16).to_le_bytes());
            }
            _ => {
                self.0.push(0x04);
                self.0.extend_from_slice(&delta.to_le_bytes());
            }
        }
        self
    }

    /// CFA = `reg` + `offset`.
    pub fn def_cfa(&mut self, reg: u16, offset: u64) -> &mut Self {
        self.0.push(0x0c);
        uleb(&mut self.0, reg.into());
        uleb(&mut self.0, offset);
        self
    }

    /// CFA uses `reg`; the offset stays.
    pub fn def_cfa_register(&mut self, reg: u16) -> &mut Self {
        self.0.push(0x0d);
        uleb(&mut self.0, reg.into());
        self
    }

    /// CFA uses `offset`; the register stays.
    pub fn def_cfa_offset(&mut self, offset: u64) -> &mut Self {
        self.0.push(0x0e);
        uleb(&mut self.0, offset);
        self
    }

    /// `reg` is saved at CFA + `factored` * data alignment.
    pub fn offset(&mut self, reg: u16, factored: u64) -> &mut Self {
        if reg < 0x40 {
            self.0.push(0x80 | reg as u8);
        } else {
            self.0.push(0x05);
            uleb(&mut self.0, reg.into());
        }
        uleb(&mut self.0, factored);
        self
    }

    /// `reg` has its value from the CIE's initial instructions again.
    pub fn restore(&mut self, reg: u16) -> &mut Self {
        if reg < 0x40 {
            self.0.push(0xc0 | reg as u8);
        } else {
            self.0.push(0x06);
            uleb(&mut self.0, reg.into());
        }
        self
    }

    pub fn remember_state(&mut self) -> &mut Self {
        self.0.push(0x0a);
        self
    }

    pub fn restore_state(&mut self) -> &mut Self {
        self.0.push(0x0b);
        self
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

/// A common information entry: what every FDE of a section shares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cie {
    pub code_align: u32,
    pub data_align: i32,
    pub return_address_register: u8,
    /// The rule set at a function's first instruction.
    pub initial: CfiProgram,
}

impl Cie {
    /// AArch64: instructions are 4 bytes, saves are 8 bytes, LR holds the return address,
    /// CFA = SP at entry.
    pub fn aarch64() -> Self {
        let mut initial = CfiProgram::new();
        initial.def_cfa(reg::aarch64::SP, 0);
        Self {
            code_align: 4,
            data_align: -8,
            return_address_register: reg::aarch64::LR as u8,
            initial,
        }
    }

    /// x86-64: CFA = RSP + 8 at entry, the return address is at CFA - 8.
    pub fn x86_64() -> Self {
        let mut initial = CfiProgram::new();
        initial
            .def_cfa(reg::x86_64::RSP, 8)
            .offset(reg::x86_64::RA, 1);
        Self {
            code_align: 1,
            data_align: -8,
            return_address_register: reg::x86_64::RA as u8,
            initial,
        }
    }

    /// The CIE of this target's architecture.
    #[cfg(target_arch = "aarch64")]
    pub fn host() -> Self {
        Self::aarch64()
    }

    /// The CIE of this target's architecture.
    #[cfg(target_arch = "x86_64")]
    pub fn host() -> Self {
        Self::x86_64()
    }
}

/// An `.eh_frame` section in 8-byte aligned storage, ready for
/// [`super::EhFrameRegistration::register`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EhFrameSection {
    words: Box<[u64]>,
    len: usize,
}

impl EhFrameSection {
    /// Copies `bytes` (a whole section with its zero terminator) and checks its record
    /// structure.
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let mut words = vec![0u64; bytes.len().div_ceil(8)];
        for (w, chunk) in words.iter_mut().zip(bytes.chunks(8)) {
            let mut b = [0u8; 8];
            b[..chunk.len()].copy_from_slice(chunk);
            *w = u64::from_le_bytes(b);
        }
        let s = Self {
            words: words.into_boxed_slice(),
            len: bytes.len(),
        };
        s.records()?;
        Ok(s)
    }

    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `words` holds at least `len` initialized bytes; u8 has no alignment or
        // validity requirement; the slice borrows `self`.
        unsafe { std::slice::from_raw_parts(self.words.as_ptr().cast::<u8>(), self.len) }
    }

    /// Byte offset and kind (`true` = CIE) of each record before the terminator. Fails if a
    /// record runs past the end, uses the 64-bit length form, points to no earlier CIE, or the
    /// terminator is missing.
    pub fn records(&self) -> io::Result<Vec<(usize, bool)>> {
        let b = self.bytes();
        let bad = |m: &'static str| io::Error::new(io::ErrorKind::InvalidData, m);
        let u32_at = |o: usize| -> io::Result<u32> {
            b.get(o..o + 4)
                .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
                .ok_or_else(|| bad("eh_frame record runs past the end"))
        };
        let mut out = Vec::new();
        let mut o = 0;
        loop {
            let len = u32_at(o)? as usize;
            if len == 0 {
                return Ok(out);
            }
            if len == 0xffff_ffff {
                return Err(bad("64-bit eh_frame records are not supported"));
            }
            if len < 4 || o + 4 + len > b.len() {
                return Err(bad("eh_frame record runs past the end"));
            }
            let id = u32_at(o + 4)? as usize;
            let is_cie = id == 0;
            if !is_cie {
                // The CIE pointer counts back from its own field.
                let cie = (o + 4)
                    .checked_sub(id)
                    .ok_or_else(|| bad("FDE points before the section"))?;
                if !out.contains(&(cie, true)) {
                    return Err(bad("FDE does not point to a CIE of this section"));
                }
            }
            out.push((o, is_cie));
            o += 4 + len;
        }
    }
}

/// Builds an `.eh_frame` section: one CIE, then FDEs.
#[derive(Clone, Debug)]
pub struct EhFrameBuilder {
    buf: Vec<u8>,
}

/// Pads a record to a multiple of 8 bytes with `DW_CFA_nop` and writes its length field.
fn close_record(buf: &mut Vec<u8>, start: usize) {
    while !(buf.len() - start).is_multiple_of(8) {
        buf.push(0);
    }
    let len = (buf.len() - start - 4) as u32;
    buf[start..start + 4].copy_from_slice(&len.to_le_bytes());
}

impl EhFrameBuilder {
    pub fn new(cie: &Cie) -> Self {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(&[0; 4]); // length, set by close_record
        buf.extend_from_slice(&0u32.to_le_bytes()); // CIE id
        buf.push(1); // version
        buf.extend_from_slice(b"zR\0");
        uleb(&mut buf, cie.code_align.into());
        sleb(&mut buf, cie.data_align.into());
        buf.push(cie.return_address_register);
        uleb(&mut buf, 1); // augmentation data length
        buf.push(DW_EH_PE_ABSPTR);
        buf.extend_from_slice(cie.initial.bytes());
        close_record(&mut buf, 0);
        Self { buf }
    }

    /// Adds an FDE for `[pc_begin, pc_begin + pc_len)` with `program` after the CIE's rules.
    pub fn add_fde(&mut self, pc_begin: usize, pc_len: usize, program: &CfiProgram) -> &mut Self {
        let start = self.buf.len();
        self.buf.extend_from_slice(&[0; 4]);
        // Distance from this field back to the CIE at offset 0.
        self.buf
            .extend_from_slice(&((start + 4) as u32).to_le_bytes());
        self.buf.extend_from_slice(&(pc_begin as u64).to_le_bytes());
        self.buf.extend_from_slice(&(pc_len as u64).to_le_bytes());
        uleb(&mut self.buf, 0); // augmentation data length
        self.buf.extend_from_slice(program.bytes());
        close_record(&mut self.buf, start);
        self
    }

    /// Adds the zero terminator.
    pub fn finish(mut self) -> EhFrameSection {
        self.buf.extend_from_slice(&0u32.to_le_bytes());
        EhFrameSection::from_bytes(&self.buf).expect("the builder writes well-formed records")
    }
}

#[cfg(test)]
pub(super) fn uleb_for_test(v: u64) -> Vec<u8> {
    let mut o = Vec::new();
    uleb(&mut o, v);
    o
}

#[cfg(test)]
pub(super) fn sleb_for_test(v: i64) -> Vec<u8> {
    let mut o = Vec::new();
    sleb(&mut o, v);
    o
}
