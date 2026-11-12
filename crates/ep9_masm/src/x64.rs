//! An x86-64 length decoder (route UNIT-X64-DECODE): it reads one instruction and gives its
//! length and the operands that the unit check needs. It is hand-written and has no tables
//! from outside (the crate has no dependencies, H34); `tests` holds lengths from `llvm-mc`.
//!
//! ```text
//!  [legacy prefixes]* [REX] opcode [ModRM [SIB] [disp8 | disp32]] [immediate | rel8 | rel32]
//!   F0 F2 F3 2E 36 3E 26 64 65 66 67      maps: one byte, 0F, 0F 38, 0F 3A
//! ```
//!
//! The decoder knows the general-purpose, x87 and SSE instructions of the one-byte map and
//! the legacy 0F, 0F 38 and 0F 3A maps. It refuses (gives `None`) a form whose length or
//! meaning it does not know for sure, so a refusal is a finding of the unit check, never a
//! wrong length:
//!
//! - a VEX (C4, C5), EVEX (62) or XOP (8F with ModRM.reg != 0) prefix: no ep9 emitter writes
//!   them; a later one extends this decoder;
//! - an opcode that is invalid in 64-bit mode (06, 07, 0E, 16, 17, 1E, 1F, 27, 2F, 37, 3F,
//!   60, 61, 82, 9A, CE, D4, D5, D6, EA, FE /2-/7, FF /7, and the 0F rows listed in [`map_0f`]);
//! - a REX byte that is not directly before the opcode (a legacy prefix or a second REX after
//!   it: the CPU ignores the first REX, and its W bit changes an immediate size);
//! - a pc-relative branch with the 66 prefix (Intel ignores it, AMD makes the target 16-bit);
//! - a RIP-relative operand with the 67 prefix (`[eip + disp32]`: the CPU cuts the address to
//!   32 bits, so it is not the text address that the check reads);
//! - every instruction with an FS (64) or GS (65) segment override: the CPU adds the FS or GS
//!   base to a memory address (a RIP, register-base, absolute, `moffs` or string operand), so
//!   it is not the address that the check reads. No ep9 emitter writes 64 or 65;
//! - a one-byte-map opcode with 66 or 67 and F2 or F3 (decoders differ on the operand or
//!   address size);
//! - `xbegin` (C7 F8: a branch whose rel field this decoder does not report);
//! - 0F 0F (3DNow!: the opcode is after the operands), 0F 20-23 (mov to and from control and
//!   debug registers ignore the ModRM mod field), 0F 78 and 0F 79 (with 66 or F2 they are
//!   SSE4a forms with two immediates), 0F B8 without F3 (JMPE);
//! - an instruction longer than 15 bytes, or one that runs past the end of the text.

/// One decoded x86-64 instruction: what the unit check reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "aot"), allow(dead_code))]
pub(crate) struct Insn {
    /// The length in bytes, 1 to 15.
    pub len: u8,
    /// A pc-relative branch (jmp, call, jcc, loop, jrcxz): the offset of its rel field in the
    /// instruction and the field size, 1 or 4. The field is the last part of the instruction,
    /// so the target is the instruction end plus the field value.
    pub rel: Option<(u8, u8)>,
    /// A RIP-relative memory operand (ModRM mod = 00, rm = 101): the offset of its disp32 in
    /// the instruction. The address is the instruction end plus the disp32 (an immediate can
    /// follow the disp32).
    pub rip: Option<u8>,
    /// An absolute memory operand: `moffs` (A0-A3) or a SIB byte with no base and mod = 00
    /// (`[disp32]` or `[index * scale + disp32]`).
    pub abs: bool,
}

/// The legacy prefixes: lock, repne, rep, the 6 segment overrides, operand size, address size.
const LEGACY: [u8; 11] = [
    0xf0, 0xf2, 0xf3, 0x2e, 0x36, 0x3e, 0x26, 0x64, 0x65, 0x66, 0x67,
];

/// The longest x86-64 instruction (Intel SDM vol. 2 section 2.3.11).
const MAX_LEN: usize = 15;

/// What follows an opcode.
#[derive(Clone, Copy)]
enum Ops {
    /// Nothing.
    Bare,
    /// A ModRM operand, then `imm` immediate bytes.
    ModRm { imm: u8 },
    /// `imm` immediate bytes and no ModRM.
    Imm(u8),
    /// A pc-relative branch with a rel field of 1 or 4 bytes.
    Rel(u8),
    /// A `moffs` address of 8 bytes (4 with the 67 prefix).
    Moffs,
}

/// Decodes the instruction at `at`. `None` when the bytes are not an instruction this decoder
/// knows (see the module text) or the instruction runs past the end of `text`.
pub(crate) fn decode(text: &[u8], at: usize) -> Option<Insn> {
    let b = text.get(at..)?;
    let b = &b[..b.len().min(MAX_LEN)];
    let mut i = 0;
    let (mut opsize16, mut addr32, mut rep, mut repne) = (false, false, false, false);
    while let Some(&p) = b.get(i) {
        if !LEGACY.contains(&p) {
            break;
        }
        if p == 0x64 || p == 0x65 {
            // FS or GS: the address is the segment base plus the operand (Codex M8-P2 r2).
            return None;
        }
        opsize16 |= p == 0x66;
        addr32 |= p == 0x67;
        rep |= p == 0xf3;
        repne |= p == 0xf2;
        i += 1;
    }
    let mut rex = 0u8;
    if let Some(&r) = b.get(i)
        && r & 0xf0 == 0x40
    {
        rex = r;
        i += 1;
    }
    let op = *b.get(i)?;
    i += 1;
    if rex != 0 && (LEGACY.contains(&op) || op & 0xf0 == 0x40) {
        return None;
    }
    let w = rex & 8 != 0;
    // Iz: 4 bytes, 2 with the 66 prefix (REX.W wins over 66).
    let iz = if opsize16 && !w { 2 } else { 4 };
    let ops = if op == 0x0f {
        let op2 = *b.get(i)?;
        i += 1;
        match op2 {
            0x38 => {
                i += 1; // the third opcode byte
                Ops::ModRm { imm: 0 }
            }
            0x3a => {
                i += 1;
                Ops::ModRm { imm: 1 }
            }
            0xb8 if !rep => return None,
            _ => map_0f(op2)?,
        }
    } else if (opsize16 || addr32) && (rep || repne) {
        // 66 or 67 with F2 or F3 in the one-byte map: llvm-mc then ignores the 66 or 67 (an
        // imm32, a moffs64), the SDM reads it as the operand or address size (an imm16, a
        // moffs32). The decoder does not choose.
        return None;
    } else {
        map_1(op, w, iz)?
    };
    let (mut rel, mut rip, mut abs) = (None, None, false);
    let imm = match ops {
        Ops::Bare => 0,
        Ops::Imm(n) => n,
        Ops::Rel(n) => {
            if opsize16 {
                return None;
            }
            rel = Some((i as u8, n));
            n
        }
        Ops::Moffs => {
            abs = true;
            if addr32 { 4 } else { 8 }
        }
        Ops::ModRm { imm } => {
            let m = *b.get(i)?;
            let reg = (m >> 3) & 7;
            let imm = match op {
                // test r/m, imm (/0, /1); the other rows of groups 3 have no immediate.
                0xf6 => u8::from(reg < 2),
                0xf7 if reg < 2 => iz,
                0xf7 => 0,
                // XOP, xbegin, and the invalid rows of groups 4 and 5.
                0x8f if reg != 0 => return None,
                0xc7 if m == 0xf8 => return None,
                0xfe if reg >= 2 => return None,
                0xff if reg == 7 => return None,
                _ => imm,
            };
            let (n, disp_rip, no_base) = modrm(b, i)?;
            if addr32 && disp_rip.is_some() {
                return None;
            }
            rip = disp_rip.map(|d| d as u8);
            abs = no_base;
            i += n;
            imm
        }
    };
    let len = i + usize::from(imm);
    (len <= b.len()).then_some(Insn {
        len: len as u8,
        rel,
        rip,
        abs,
    })
}

/// The ModRM byte at `i` and what follows it: the byte count (ModRM, SIB, displacement), the
/// offset of a RIP-relative disp32, and whether the operand has no base register (absolute).
/// REX.B does not change the two special forms (rm = 101 and SIB base = 101 with mod = 00).
fn modrm(b: &[u8], i: usize) -> Option<(usize, Option<usize>, bool)> {
    let m = *b.get(i)?;
    let (md, rm) = (m >> 6, m & 7);
    if md == 3 {
        return Some((1, None, false));
    }
    if rm == 4 {
        let sib = *b.get(i + 1)?;
        return Some(match md {
            0 if sib & 7 == 5 => (6, None, true),
            0 => (2, None, false),
            1 => (3, None, false),
            _ => (6, None, false),
        });
    }
    Some(match md {
        0 if rm == 5 => (5, Some(i + 1), false),
        0 => (1, None, false),
        1 => (2, None, false),
        _ => (5, None, false),
    })
}

/// The one-byte opcode map in 64-bit mode (Intel SDM vol. 2 appendix A, table A-2). `None` for
/// an opcode that is invalid in 64-bit mode or that this decoder refuses; a legacy prefix or a
/// REX byte here (after a REX byte) is refused by the caller first.
fn map_1(op: u8, w: bool, iz: u8) -> Option<Ops> {
    use Ops::*;
    let m = ModRm { imm: 0 };
    Some(match op {
        // The 8 ALU rows: r/m and reg forms, then AL, imm8 and eAX, imm.
        0x00..=0x3f => match op & 7 {
            0..=3 => m,
            4 => Imm(1),
            5 => Imm(iz),
            _ => return None,
        },
        0x50..=0x5f => Bare,
        0x63 => m,
        0x68 => Imm(iz),
        0x69 => ModRm { imm: iz },
        0x6a => Imm(1),
        0x6b => ModRm { imm: 1 },
        0x6c..=0x6f => Bare,
        0x70..=0x7f => Rel(1),
        0x80 | 0x83 => ModRm { imm: 1 },
        0x81 => ModRm { imm: iz },
        0x84..=0x8f => m,
        0x90..=0x99 | 0x9b..=0x9f => Bare,
        0xa0..=0xa3 => Moffs,
        0xa4..=0xa7 | 0xaa..=0xaf => Bare,
        0xa8 => Imm(1),
        0xa9 => Imm(iz),
        0xb0..=0xb7 => Imm(1),
        0xb8..=0xbf => Imm(if w { 8 } else { iz }),
        0xc0 | 0xc1 | 0xc6 => ModRm { imm: 1 },
        0xc2 | 0xca => Imm(2),
        0xc3 | 0xc9 | 0xcb | 0xcc | 0xcf => Bare,
        0xc7 => ModRm { imm: iz },
        0xc8 => Imm(3),
        0xcd => Imm(1),
        0xd0..=0xd3 | 0xd8..=0xdf => m,
        0xd7 => Bare,
        0xe0..=0xe3 | 0xeb => Rel(1),
        0xe4..=0xe7 => Imm(1),
        0xe8 | 0xe9 => Rel(4),
        0xec..=0xef | 0xf1 | 0xf4 | 0xf5 | 0xf8..=0xfd => Bare,
        0xf6 | 0xf7 | 0xfe | 0xff => m,
        // 60 61 62 (EVEX) 82 9A C4 C5 (VEX) CE D4 D5 D6 EA, and the prefixes.
        _ => return None,
    })
}

/// The two-byte map 0F (Intel SDM vol. 2 table A-3), without 0F 38 and 0F 3A. `None` for an
/// invalid or refused opcode: 04, 0A, 0C, 0F (3DNow!), 20-27, 36, 39, 3B-3F, 78-7B, A6, A7.
fn map_0f(op: u8) -> Option<Ops> {
    use Ops::*;
    let m = ModRm { imm: 0 };
    Some(match op {
        0x00..=0x03 | 0x0d | 0x10..=0x1f | 0x28..=0x2f | 0x40..=0x6f => m,
        0x05..=0x09 | 0x0b | 0x0e | 0x30..=0x35 | 0x37 | 0x77 => Bare,
        0x70..=0x73 => ModRm { imm: 1 },
        0x74..=0x76 | 0x7c..=0x7f | 0x90..=0x9f => m,
        0x80..=0x8f => Rel(4),
        0xa0..=0xa2 | 0xa8..=0xaa | 0xc8..=0xcf => Bare,
        0xa3 | 0xa5 | 0xab | 0xad..=0xb9 | 0xbb..=0xc1 | 0xc3 | 0xc7 | 0xd0..=0xff => m,
        0xa4 | 0xac | 0xba | 0xc2 | 0xc4..=0xc6 => ModRm { imm: 1 },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lengths from `llvm-mc` (see `tests/golden/x64-lengths.txt`): every line is the bytes
    /// of one instruction that llvm-mc encoded, and the decoder gives that length, or `None`
    /// for the 8 lines with an FS (64) override (refused since Codex M8-P2 r2).
    #[test]
    fn the_decoder_gives_the_llvm_mc_length_of_each_corpus_instruction() {
        let corpus = include_str!("../tests/golden/x64-lengths.txt");
        let (mut n, mut segs) = (0, 0);
        for line in corpus
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let (hex, asm) = line.split_once(" ; ").expect("bytes ; asm");
            let bytes: Vec<u8> = hex
                .split_whitespace()
                .map(|h| u8::from_str_radix(h, 16).expect("hex byte"))
                .collect();
            let insn = decode(&bytes, 0);
            // An FS or GS override is refused (see the module text); its length is not read.
            let seg = bytes
                .iter()
                .take_while(|p| LEGACY.contains(p))
                .any(|&p| p == 0x64 || p == 0x65);
            segs += usize::from(seg);
            assert_eq!(
                insn.map(|i| usize::from(i.len)),
                (!seg).then_some(bytes.len()),
                "{asm}: {hex}"
            );
            // The instruction followed by more bytes has the same length; cut short, none.
            let mut longer = bytes.clone();
            longer.extend_from_slice(&[0x90; 16]);
            assert_eq!(decode(&longer, 0), insn, "{asm}");
            assert_eq!(decode(&bytes[..bytes.len() - 1], 0), None, "{asm}");
            n += 1;
        }
        assert!(n > 300, "{n} corpus lines");
        assert_eq!(segs, 8);
    }

    #[test]
    fn the_decoder_reports_branches_rip_and_absolute_operands() {
        let d = |b: &[u8]| decode(b, 0).unwrap();
        // jmp rel8, jne rel32, call rel32.
        assert_eq!(d(&[0xeb, 0xfe]).rel, Some((1, 1)));
        assert_eq!(d(&[0x0f, 0x85, 0, 0, 0, 0]).rel, Some((2, 4)));
        assert_eq!(d(&[0xe8, 0, 0, 0, 0]).rel, Some((1, 4)));
        // mov rax, [rip + 0]; cmp dword ptr [rip + 0], 1 (an imm8 after the disp32).
        let r = d(&[0x48, 0x8b, 0x05, 0, 0, 0, 0]);
        assert_eq!((r.len, r.rip, r.abs), (7, Some(3), false));
        let r = d(&[0x83, 0x3d, 0, 0, 0, 0, 1]);
        assert_eq!((r.len, r.rip), (7, Some(2)));
        // mov rax, [disp32] (SIB, no base); movabs eax, [moffs64]; mov r13 base needs disp8.
        assert!(d(&[0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]).abs);
        assert!(d(&[0xa1, 0, 0, 0, 0, 0, 0, 0, 0]).abs);
        assert_eq!(d(&[0x49, 0x8b, 0x45, 0]).len, 4);
        // A REX byte not right before the opcode, VEX, EVEX, a 66 branch, xbegin, XOP,
        // 3DNow!, mov from cr0, 66 or 67 with F3 in the one-byte map: refused.
        for b in [
            &[0x48, 0x66, 0x8b, 0xc0][..],
            &[0x48, 0x48, 0x8b, 0xc0],
            &[0xc5, 0xf8, 0x77],
            &[0x62, 0xf1, 0x7c, 0x48, 0x28, 0xc1],
            &[0x66, 0xe9, 0, 0],
            &[0xc7, 0xf8, 0, 0, 0, 0],
            &[0x8f, 0xe9, 0x78, 0x01, 0xc8],
            &[0x0f, 0x0f, 0xc1, 0xb4],
            &[0x0f, 0x20, 0xc0],
            &[0x66, 0xf3, 0x1d, 0x34, 0x12, 0, 0],
            &[0x67, 0xf3, 0xa0, 1, 2, 3, 4, 5, 6, 7, 8],
        ] {
            assert_eq!(decode(b, 0), None, "{b:02x?}");
        }
        // 15 bytes is the limit: 14 prefixes and a nop decode, 15 and a nop do not.
        let mut p = vec![0x66; 14];
        p.push(0x90);
        assert_eq!(d(&p).len, 15);
        let mut p = vec![0x66; 15];
        p.push(0x90);
        assert_eq!(decode(&p, 0), None);
    }

    /// Codex M8-P2 r1 H3: `67 8b 05 d32` is `mov eax, [eip + d32]` (llvm-mc 23.1.4:
    /// `movl (%eip), %eax`; Intel SDM vol. 2 section 2.2.1.6: with the 67 prefix the
    /// RIP-relative address is cut to 32 bits). The text is mapped above 4 GiB, so that address
    /// is not the text byte that the check reads: the decoder refuses it. The old decoder gave
    /// `rip: Some(2)`, so `check_text` passed the load. Without 67 the form still decodes, and
    /// 67 with a register base (`[r14d + 80]`) is not RIP-relative and stays.
    #[test]
    fn the_decoder_refuses_a_rip_relative_operand_with_the_67_prefix() {
        for b in [
            &[0x67, 0x8b, 0x05, 0, 0, 0, 0][..],
            &[0x67, 0x48, 0x8d, 0x05, 0, 0, 0, 0],
            &[0x67, 0x83, 0x3d, 0, 0, 0, 0, 1],
            &[0x67, 0xff, 0x15, 0, 0, 0, 0],
        ] {
            assert_eq!(decode(b, 0), None, "{b:02x?}");
        }
        let r = decode(&[0x8b, 0x05, 0, 0, 0, 0], 0).unwrap();
        assert_eq!((r.len, r.rip), (6, Some(2)));
        let r = decode(&[0x67, 0x49, 0x8b, 0x46, 0x50], 0).unwrap();
        assert_eq!((r.len, r.rip), (5, None));
    }

    /// Codex M8-P2 r2 R2-1: `64 8b 05 d32` is `mov eax, fs:[rip + d32]`. The CPU adds the FS
    /// base (GS for 65), so the address is not the text byte that the check reads. No ep9
    /// emitter writes 64 or 65, so the decoder refuses every instruction with one: a RIP,
    /// register-base, absolute, `moffs` or string operand. The old decoder dropped the prefix
    /// and gave `rip: Some(2)` for the first form. Without the prefix each form decodes.
    #[test]
    fn the_decoder_refuses_an_fs_or_gs_override() {
        for b in [
            &[0x64, 0x8b, 0x05, 0, 0, 0, 0][..],
            &[0x65, 0x48, 0x8b, 0x05, 0, 0, 0, 0],
            &[0x65, 0x49, 0x8b, 0x46, 0x50],
            &[0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0],
            &[0x64, 0xa1, 0, 0, 0, 0, 0, 0, 0, 0],
            &[0x65, 0xac],
            &[0x66, 0x64, 0x8b, 0x05, 0, 0, 0, 0],
        ] {
            assert_eq!(decode(b, 0), None, "{b:02x?}");
            let plain: Vec<u8> = b
                .iter()
                .copied()
                .filter(|&p| p != 0x64 && p != 0x65)
                .collect();
            assert!(decode(&plain, 0).is_some(), "{plain:02x?}");
        }
    }

    /// Codex M8-P2 r1 H1 (premise check; the premise does not hold). `push imm` (68) has a
    /// 64-bit default operand size in 64-bit mode; 66 makes it 16-bit (imm16), and REX.W wins
    /// over 66 (Intel SDM vol. 1 table 3-4 and vol. 2 section 2.2.1.2: with REX.W = 1 the 66
    /// prefix is ignored), so `66 48 68` takes an imm32, as with no prefix. llvm-mc 23.1.4 gives
    /// `pushq $0x7feb0000` for `66 48 68 00 00 eb 7f`, and Zydis (the "defaults to 64-bit"
    /// operand size map: 64 66 W1 -> 64) agrees. So the hidden `jmp` of the review is part of
    /// the immediate on the CPU too, and the decoder length (7) is right.
    #[test]
    fn the_decoder_sizes_push_imm_by_rex_w_over_66() {
        let d = |b: &[u8]| decode(b, 0).map(|i| i.len);
        assert_eq!(
            d(&[0x66, 0x48, 0x68, 0x00, 0x00, 0xeb, 0x7f, 0xc3]),
            Some(7)
        );
        assert_eq!(d(&[0x48, 0x68, 0, 0, 0, 0]), Some(6));
        assert_eq!(d(&[0x68, 0, 0, 0, 0]), Some(5));
        assert_eq!(d(&[0x66, 0x68, 0, 0]), Some(4));
        assert_eq!(d(&[0x66, 0x6a, 0]), Some(3));
    }
}
