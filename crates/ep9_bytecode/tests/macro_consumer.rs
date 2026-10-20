//! `for_each_opcode!` from outside the crate (gate "opcode-list"): an interpreter-style
//! consumer generates a per-opcode operand decoder from the macro alone, and it must agree
//! with the table-driven decoder on every instruction of many units.

mod common;

use ep9_bytecode::*;

/// What the ep9_vm DSL needs per handler: operand names bound to decoded values, in order.
macro_rules! generated_decoder {
    ( []
      $( $(#[$doc:meta])* $name:ident = $code:literal, $snake:ident, acc: $acc:ident,
         flags: [$($flag:ident),*],
         operands: ($($oname:ident : $okind:ident $(($oparam:tt))?),*); )*
    ) => {
        /// Decodes the operands of the instruction at `pc` (after any prefix) at `scale`.
        fn decode_generated(code: &[u8], pc: usize, scale: OperandScale) -> (Opcode, Vec<(&'static str, i64)>) {
            let op = Opcode::from_u8(code[pc]).unwrap();
            let mut at = pc + 1;
            let mut out = Vec::new();
            match op {
                $( Opcode::$name => {
                    $(
                        let kind = OperandKind::$okind;
                        let $oname = read_operand(&code[at..], kind, scale);
                        at += kind.size(scale);
                        out.push((stringify!($oname), $oname));
                    )*
                } )*
            }
            let _ = at;
            (op, out)
        }

        /// Every handler name an interpreter would need, in opcode order.
        const HANDLER_NAMES: &[&str] = &[ $( stringify!($snake) ),* ];
    };
}

ep9_bytecode::for_each_opcode!(generated_decoder []);

#[test]
fn generated_decoder_agrees_with_table_decoder() {
    let mut checked = 0usize;
    for seed in 0..200 {
        let unit = common::random_unit(seed).build_in_order().unwrap();
        for f in &unit.functions {
            for insn in Instructions::new(&f.bytecode) {
                let insn = insn.unwrap();
                let pc = insn.offset as usize + usize::from(insn.scale != OperandScale::Single);
                let (op, ops) = decode_generated(&f.bytecode, pc, insn.scale);
                assert_eq!(op, insn.opcode);
                let info = op.info();
                assert_eq!(ops.len(), info.operands.len());
                for (k, (name, v)) in ops.iter().enumerate() {
                    assert_eq!(*name, info.operands[k].name);
                    assert_eq!(*v, insn.operand(k), "{op} operand {k}");
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 1000);
}

#[test]
fn handler_names_match_the_table() {
    assert_eq!(HANDLER_NAMES.len(), OPCODE_COUNT);
    for (i, name) in HANDLER_NAMES.iter().enumerate() {
        // The table strips the raw-identifier prefix (`r#mod` -> `mod`).
        assert_eq!(name.trim_start_matches("r#"), OPCODES[i].snake);
    }
}
