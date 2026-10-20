//! A small assembler for kernels and tests.

use crate::Op;
use crate::dsl::OperandKind;
use crate::program::{Func, Program};

#[derive(Clone, Copy, Debug)]
pub struct Label(usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FuncId(pub u8);

pub struct Asm {
    code: Vec<u8>,
    consts: Vec<i64>,
    funcs: Vec<Func>,
    current: Option<usize>,
    labels: Vec<Option<usize>>,
    /// (offset of the i16 operand, start of its instruction, label)
    fixups: Vec<(usize, usize, Label)>,
    throw_off: u32,
}

impl Default for Asm {
    fn default() -> Self {
        Self::new()
    }
}

impl Asm {
    /// Starts a program with the entry stub (function 0): `Call 1, r0, 0; Halt`.
    /// The next `func` call declares function 1, the program's main function.
    pub fn new() -> Asm {
        let mut a = Asm {
            code: Vec::new(),
            consts: Vec::new(),
            funcs: Vec::new(),
            current: None,
            labels: Vec::new(),
            fixups: Vec::new(),
            throw_off: 0,
        };
        let entry = a.func(1);
        a.begin(entry);
        a.op(Op::Call, &[1, 0, 0]);
        a.throw_off = a.code.len() as u32;
        a.op(Op::Halt, &[]);
        a.end();
        a
    }

    /// Declares a function. Its code comes later, between `begin` and `end`.
    pub fn func(&mut self, nregs: u32) -> FuncId {
        let id = self.funcs.len();
        assert!(id < 256);
        self.funcs.push(Func {
            offset: 0,
            len: 0,
            nregs,
        });
        FuncId(id as u8)
    }

    pub fn begin(&mut self, f: FuncId) {
        assert!(self.current.is_none(), "begin inside a function");
        self.current = Some(f.0 as usize);
        self.funcs[f.0 as usize].offset = self.code.len() as u32;
    }

    pub fn end(&mut self) {
        let f = self.current.take().expect("end outside a function");
        self.funcs[f].len = self.code.len() as u32 - self.funcs[f].offset;
    }

    /// Adds a constant and returns its index.
    pub fn konst(&mut self, v: i64) -> i64 {
        if let Some(i) = self.consts.iter().position(|&c| c == v) {
            return i as i64;
        }
        self.consts.push(v);
        assert!(self.consts.len() <= 256);
        (self.consts.len() - 1) as i64
    }

    pub fn label(&mut self) -> Label {
        self.labels.push(None);
        Label(self.labels.len() - 1)
    }

    pub fn bind(&mut self, l: Label) {
        assert!(self.labels[l.0].is_none(), "label bound twice");
        self.labels[l.0] = Some(self.code.len());
    }

    /// Emits one instruction. Jump operands go through `jump`.
    pub fn op(&mut self, op: Op, operands: &[i64]) {
        assert!(self.current.is_some(), "op outside a function");
        let info = op.info();
        assert_eq!(info.operands.len(), operands.len(), "{op:?}: operand count");
        self.code.push(op as u8);
        for (kind, &v) in info.operands.iter().zip(operands) {
            match kind {
                OperandKind::Off => panic!("{op:?}: use Asm::jump"),
                OperandKind::Imm => {
                    assert!((-128..=127).contains(&v), "{op:?}: imm {v} out of range");
                    self.code.push(v as i8 as u8);
                }
                _ => {
                    assert!((0..=255).contains(&v), "{op:?}: operand {v} out of range");
                    self.code.push(v as u8);
                }
            }
        }
    }

    /// Emits a jump to a label (bound now or later).
    pub fn jump(&mut self, op: Op, l: Label) {
        assert_eq!(
            op.info().operands,
            &[OperandKind::Off],
            "{op:?} is not a jump"
        );
        let start = self.code.len();
        self.code.push(op as u8);
        self.fixups.push((self.code.len(), start, l));
        self.code.extend_from_slice(&[0, 0]);
    }

    pub fn finish(mut self) -> Program {
        assert!(self.current.is_none(), "unfinished function");
        for &(at, start, l) in &self.fixups {
            let target = self.labels[l.0].expect("unbound label");
            let off = i16::try_from(target as i64 - start as i64).expect("jump too far");
            self.code[at..at + 2].copy_from_slice(&off.to_le_bytes());
        }
        Program {
            code: self.code,
            consts: self.consts,
            funcs: self.funcs,
            throw_off: self.throw_off,
        }
    }
}
