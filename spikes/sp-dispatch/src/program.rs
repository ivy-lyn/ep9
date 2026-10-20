//! Bytecode program, its file format (shared with cgoto.c) and the verifier.
//!
//! File format `.ep9d`, all integers little-endian:
//!
//! ```text
//! "EP9D" u32:version(1)
//! u32:code_len   u8[code_len]
//! u32:nconsts    i64[nconsts]
//! u32:nfuncs     { u32:offset u32:len u32:nregs }[nfuncs]
//! u32:throw_off
//! ```
//!
//! Function 0 is the entry stub: `Call 1, r0, 0; Halt`. `throw_off` points at that Halt.

use crate::dsl::OperandKind;
use crate::ops::HEADER;
use crate::vm::STACK_SLOTS;
use crate::{OPS, Op};

pub const MAGIC: &[u8; 4] = b"EP9D";
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Func {
    pub offset: u32,
    pub len: u32,
    pub nregs: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub code: Vec<u8>,
    pub consts: Vec<i64>,
    pub funcs: Vec<Func>,
    pub throw_off: u32,
}

/// A program that passed `Program::verify`. The interpreters read operands without bounds
/// checks, so they take only this type.
#[derive(Debug)]
pub struct Verified(Program);

impl Verified {
    pub fn program(&self) -> &Program {
        &self.0
    }
}

impl Program {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        b.extend_from_slice(&VERSION.to_le_bytes());
        b.extend_from_slice(&(self.code.len() as u32).to_le_bytes());
        b.extend_from_slice(&self.code);
        b.extend_from_slice(&(self.consts.len() as u32).to_le_bytes());
        for c in &self.consts {
            b.extend_from_slice(&c.to_le_bytes());
        }
        b.extend_from_slice(&(self.funcs.len() as u32).to_le_bytes());
        for f in &self.funcs {
            for v in [f.offset, f.len, f.nregs] {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        b.extend_from_slice(&self.throw_off.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<Program, String> {
        let mut r = Reader { b, at: 0 };
        if r.take(4)? != MAGIC {
            return Err("bad magic".into());
        }
        if r.u32()? != VERSION {
            return Err("bad version".into());
        }
        let n = r.u32()? as usize;
        let code = r.take(n)?.to_vec();
        let n = r.u32()? as usize;
        let mut consts = Vec::with_capacity(n);
        for _ in 0..n {
            consts.push(i64::from_le_bytes(r.take(8)?.try_into().unwrap()));
        }
        let n = r.u32()? as usize;
        let mut funcs = Vec::with_capacity(n);
        for _ in 0..n {
            funcs.push(Func {
                offset: r.u32()?,
                len: r.u32()?,
                nregs: r.u32()?,
            });
        }
        let throw_off = r.u32()?;
        if r.at != b.len() {
            return Err("trailing bytes".into());
        }
        Ok(Program {
            code,
            consts,
            funcs,
            throw_off,
        })
    }

    /// Checks everything the handlers rely on:
    /// - every opcode is defined and every instruction fits inside its function;
    /// - registers < nregs, constant and function indexes inside their tables;
    /// - Call argument windows inside the frame, and argc <= callee nregs;
    /// - jump targets are instruction starts inside the same function;
    /// - the last instruction of a function cannot fall through;
    /// - the entry function has no Return (its frame has no caller);
    /// - the throw stub is a Halt in the entry function.
    pub fn verify(self) -> Result<Verified, String> {
        if self.funcs.is_empty() || self.funcs.len() > 256 {
            return Err("need 1..=256 functions".into());
        }
        if self.consts.len() > 256 {
            return Err("too many constants".into());
        }
        for (fi, f) in self.funcs.iter().enumerate() {
            let err = |m: String| Err(format!("func {fi}: {m}"));
            let (start, end) = (f.offset as usize, f.offset as usize + f.len as usize);
            if end > self.code.len() || f.len == 0 {
                return err("code range out of bounds or empty".into());
            }
            if f.nregs == 0 || f.nregs > 256 || HEADER + f.nregs as usize > STACK_SLOTS {
                return err(format!("bad register count {}", f.nregs));
            }
            // Pass 1: instruction starts.
            let mut starts = vec![false; f.len as usize];
            let mut at = start;
            let mut last = None;
            while at < end {
                let Some(op) = Op::from_u8(self.code[at]) else {
                    return err(format!("bad opcode {} at {at}", self.code[at]));
                };
                let len = op.info().insn_len();
                if at + len > end {
                    return err(format!("{op:?} at {at} runs past the function end"));
                }
                // Only Call writes the frame header that Return reads. The entry frame has
                // none: its zero slots would send Return to code offset 0.
                if fi == 0 && op == Op::Return {
                    return err(format!("Return at {at}: the entry frame has no caller"));
                }
                starts[at - start] = true;
                last = Some(op);
                at += len;
            }
            if !matches!(last, Some(Op::Halt | Op::Jump | Op::JumpLoop | Op::Return)) {
                return err(format!("last instruction {last:?} can fall through"));
            }
            // Pass 2: operands.
            let mut at = start;
            while at < end {
                let op = Op::from_u8(self.code[at]).unwrap();
                let info = op.info();
                let mut o = at + 1;
                let mut vals = [0i64; 3];
                for (k, kind) in info.operands.iter().enumerate() {
                    let v = match kind {
                        OperandKind::Off => {
                            i16::from_le_bytes([self.code[o], self.code[o + 1]]) as i64
                        }
                        OperandKind::Imm => self.code[o] as i8 as i64,
                        _ => self.code[o] as i64,
                    };
                    vals[k] = v;
                    let bad = match kind {
                        OperandKind::Reg => v >= f.nregs as i64,
                        OperandKind::Const => v >= self.consts.len() as i64,
                        OperandKind::Func => v >= self.funcs.len() as i64,
                        OperandKind::Imm | OperandKind::Cnt => false,
                        OperandKind::Off => {
                            let t = at as i64 + v;
                            t < start as i64 || t >= end as i64 || !starts[t as usize - start]
                        }
                    };
                    if bad {
                        return err(format!("{op:?} at {at}: bad {kind:?} operand {v}"));
                    }
                    o += kind.size();
                }
                if op == Op::Call {
                    let (callee, args, argc) = (vals[0] as usize, vals[1], vals[2]);
                    if args + argc > f.nregs as i64 || argc > self.funcs[callee].nregs as i64 {
                        return err(format!("Call at {at}: bad argument window"));
                    }
                }
                at += info.insn_len();
            }
        }
        let e = &self.funcs[0];
        let t = self.throw_off as usize;
        if t < e.offset as usize
            || t >= (e.offset + e.len) as usize
            || self.code[t] != Op::Halt as u8
        {
            return Err("throw_off must point at a Halt in function 0".into());
        }
        // The Halt must be an instruction start: decode function 0 again.
        let mut at = e.offset as usize;
        while at < t {
            at += OPS[self.code[at] as usize].insn_len();
        }
        if at != t {
            return Err("throw_off is not an instruction start".into());
        }
        Ok(Verified(self))
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.b.get(self.at..self.at + n).ok_or("truncated file")?;
        self.at += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}
