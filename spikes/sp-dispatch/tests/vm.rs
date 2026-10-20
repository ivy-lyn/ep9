//! Correctness of every expansion against the reference models, and verifier rejections.
//! Debug builds run the threaded variants too, so inputs stay small: a debug sibling-call
//! chain may grow the native stack (the plan's reason for match in debug builds).

use sp_dispatch::Op::*;
use sp_dispatch::asm::Asm;
use sp_dispatch::kernels::{self, KERNELS};
use sp_dispatch::program::{Func, Program};
use sp_dispatch::vm::{self, ERR_STACK_OVERFLOW, Variant};
use sp_dispatch::{OPS, Op};

fn variants() -> Vec<Variant> {
    let mut v = Variant::ALL.to_vec();
    v.push(Variant::MatchCounting);
    v
}

#[test]
fn kernels_match_reference() {
    for k in KERNELS {
        let sizes: &[i64] = if k.name == "call" {
            &[0, 1, 2, 7, 12]
        } else {
            &[0, 1, 3, 40]
        };
        for &n in sizes {
            let prog = (k.build)(n).verify().unwrap();
            let want = (k.reference)(n);
            for v in variants() {
                let o = vm::run(&prog, v);
                assert_eq!(
                    (o.result, o.interrupts, o.slow_paths, o.error),
                    (want.0, want.1, want.2, 0),
                    "{} n={n} {v:?}",
                    k.name
                );
            }
        }
    }
}

#[test]
fn cold_paths() {
    let prog = kernels::build_overflow().verify().unwrap();
    for v in variants() {
        let o = vm::run(&prog, v);
        assert_eq!((o.result, o.slow_paths, o.error), (-2, 3, 0), "{v:?}");
    }
}

/// 9,363 dispatches (one Call per VM frame). In a debug build every sibling dispatch is a
/// real call (no tail call without optimization), so the sibling variant would overflow the
/// 2 MiB test thread stack here. The plan's rule: debug builds use match. `become` guarantees the tail call
/// even without optimization, so it runs here in debug builds too (D8). Release runs all.
#[test]
fn runaway_recursion_throws() {
    let prog = kernels::build_runaway().verify().unwrap();
    let vs: Vec<Variant> = if cfg!(debug_assertions) {
        // Match, plus Become when it is built; never Sibling.
        Variant::ALL
            .iter()
            .copied()
            .filter(|v| *v != Variant::Sibling)
            .chain([Variant::MatchCounting])
            .collect()
    } else {
        variants()
    };
    for v in vs {
        let o = vm::run(&prog, v);
        assert_eq!((o.result, o.error), (i64::MIN, ERR_STACK_OVERFLOW), "{v:?}");
    }
}

#[test]
fn file_format_round_trip() {
    let p = (KERNELS[1].build)(5);
    assert_eq!(Program::from_bytes(&p.to_bytes()).unwrap(), p);
    let mut bad = p.to_bytes();
    bad.push(0);
    assert!(Program::from_bytes(&bad).is_err());
}

#[test]
fn opcode_table() {
    assert_eq!(OPS.len(), 20);
    for (i, info) in OPS.iter().enumerate() {
        assert_eq!(info.op as usize, i);
        assert_eq!(Op::from_u8(i as u8), Some(info.op));
    }
    assert_eq!(Op::from_u8(20), None);
    assert!(Halt.info().exit && !Add.info().exit);
    assert!(Add.info().cold && !Ldar.info().cold);
    assert_eq!(Call.info().insn_len(), 4);
    assert_eq!(JumpLoop.info().insn_len(), 3);
}

/// One function body `f(asm)` after the entry stub; returns the verifier result.
fn verify_main(nregs: u32, f: impl FnOnce(&mut Asm)) -> Result<(), String> {
    let mut a = Asm::new();
    let main = a.func(nregs);
    a.begin(main);
    f(&mut a);
    a.end();
    a.finish().verify().map(|_| ())
}

#[test]
fn verifier_rejects() {
    // Register out of the frame.
    assert!(
        verify_main(2, |a| {
            a.op(Ldar, &[2]);
            a.op(Return, &[]);
        })
        .unwrap_err()
        .contains("Reg")
    );
    // Falls off the end.
    assert!(
        verify_main(1, |a| a.op(LdaSmi, &[1]))
            .unwrap_err()
            .contains("fall through")
    );
    // Constant index out of range.
    assert!(
        verify_main(1, |a| {
            a.op(LdaConst, &[0]);
            a.op(Return, &[]);
        })
        .is_err()
    );
    // Call argument window past the frame.
    assert!(
        verify_main(2, |a| {
            a.op(Call, &[1, 1, 2]);
            a.op(Return, &[]);
        })
        .unwrap_err()
        .contains("argument window")
    );
    // Jump into the middle of an instruction.
    let mut p = {
        let mut a = Asm::new();
        let main = a.func(1);
        a.begin(main);
        let l = a.label();
        a.bind(l);
        a.op(LdaSmi, &[0]);
        a.jump(JumpLoop, l);
        a.end();
        a.finish()
    };
    let at = p.code.len() - 2;
    p.code[at..].copy_from_slice(&(-1i16).to_le_bytes());
    assert!(p.verify().unwrap_err().contains("Off"));
    // Unknown opcode.
    let mut p = (KERNELS[0].build)(1);
    let main = p.funcs[1].offset as usize;
    p.code[main] = 200;
    assert!(p.verify().unwrap_err().contains("bad opcode"));
    // Return in the entry frame. Its header slots are zero, so Return jumped to code offset 0,
    // outside every function: LdaConst 0 with no constants (SIGSEGV), or Jump +0x7fff (a read
    // past the code buffer).
    for code in [vec![LdaConst as u8, 0], vec![Jump as u8, 0xff, 0x7f]] {
        let off = code.len() as u32;
        let mut code = code;
        code.extend_from_slice(&[Return as u8, Halt as u8]);
        let p = Program {
            code,
            consts: vec![],
            funcs: vec![Func {
                offset: off,
                len: 2,
                nregs: 1,
            }],
            throw_off: off + 1,
        };
        assert!(p.verify().unwrap_err().contains("entry frame"));
    }
    // A well-formed program passes.
    assert!(
        verify_main(1, |a| {
            a.op(LdaSmi, &[1]);
            a.op(Return, &[]);
        })
        .is_ok()
    );
}
