//! The relocation scan of unit text ("a text scan finds 0 absolute relocations").
//!
//! | Rule | AArch64 | x86-64 |
//! |---|---|---|
//! | no 8-byte window (any alignment) is within 64 KiB of a fixed address of this process (a [`Live`] point: the Cx, the runtime table and its functions, FunctionInfos, bytecode, heap cells, the RW copy, the directory) | yes | yes |
//! | no 8-byte window is an address in a placed range (a [`Placed`]: the mapped unit text, the code range) that moves with the range: read at a second placement, the window holds the same offset in the moved range | yes | yes |
//! | no constant that `movz`/`movn`/`movk` build in a register is near a [`Live`] point | yes | - |
//! | no `adrp` (a page-relative reference) | yes | - |
//! | every pc-relative form (`b`, `b.cond`, `cbz`/`cbnz`, `tbz`/`tbnz`, `adr`, `ldr` literal) stays inside its own function; no direct call (`bl`) | yes | - |
//!
//! The x86-64 code generator writes branches only through dynasm labels inside one function
//! (`jit_b::x64` rule) and has no decoder here; its text gets the window rule and the
//! file-equals-mapped rule of the load check.
//!
//! Why a placed range needs a second placement (M6-ORCH-11): the process picks the place of
//! the code range at random (a 4 GiB-aligned slot, `ep9_platform::CodeRangeConfig`), and the
//! first chunk of the range starts at the slot itself, so its address has 32 low zero bits.
//! Unit text has fixed bytes such as `00 00 00 00 e9 32 00 00` (the imm32 0 of `mov edx, 0`,
//! then `jmp rel32` 0x32), and they equal the slot address `0x32e9_0000_0000` when the range
//! happens to be there (CI 41420942032). The text bytes are fixed before the text is placed
//! (the build has no address input; LOAD copies the bytes unchanged), so such a value stays
//! the same at a second placement. A real reference into a placed range moves with it. A
//! window that hits a placed range only at the first placement is a fixed value
//! ([`Windows::fixed`]), not a finding. Limit: in one process, both readings of a text come
//! from one build, so a placed-range address that a build read from a static of this process
//! would not move either; only a build in a second process could show it. The point rule still
//! finds every fixed address of the live list.

use std::ops::Range;

/// One finding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Text offset.
    pub at: usize,
    pub what: String,
}

/// A range whose place the process picks (the mapped unit text, the code range), with the
/// start of a second placement of the same range.
#[derive(Clone, Debug)]
pub struct Placed {
    pub name: String,
    /// Where the range is (the first placement).
    pub at: Range<u64>,
    /// The start of the second placement. It differs from `at.start`.
    pub moved: u64,
}

impl Placed {
    /// The second-placement value of a reference to the first-placement address `v`.
    fn moved_value(&self, v: u64) -> u64 {
        v.wrapping_add(self.moved.wrapping_sub(self.at.start))
    }
}

/// Addresses the scan looks for: fixed addresses (with a 64 KiB margin each way) and placed
/// ranges.
#[derive(Clone, Debug, Default)]
pub struct Live {
    pub points: Vec<(String, u64)>,
    pub placed: Vec<Placed>,
}

/// The margin around each fixed address.
pub const MARGIN: u64 = 64 << 10;

impl Live {
    pub fn point(&mut self, name: &str, addr: usize) -> &mut Self {
        self.points.push((name.to_string(), addr as u64));
        self
    }

    /// Adds the placed range `at`, whose second placement starts at `moved`. Fails if the two
    /// placements start at the same address: then no window can show that it moves.
    pub fn placed(&mut self, name: &str, at: Range<usize>, moved: usize) -> Result<(), String> {
        if moved == at.start {
            return Err(format!(
                "{name}: the second placement starts at the first one ({moved:#x})"
            ));
        }
        self.placed.push(Placed {
            name: name.to_string(),
            at: at.start as u64..at.end as u64,
            moved: moved as u64,
        });
        Ok(())
    }

    /// The name of the fixed address `v` is, or is near.
    pub fn hit(&self, v: u64) -> Option<&str> {
        if v < 1 << 16 {
            return None;
        }
        self.points
            .iter()
            .find(|(_, p)| v.abs_diff(*p) < MARGIN)
            .map(|(n, _)| n.as_str())
    }
}

/// The result of the window rule.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Windows {
    /// Relocations: a window near a fixed address, or one that moves with a placed range.
    pub found: Vec<Finding>,
    /// Not relocations: a window that is in a placed range at the first placement only (the
    /// same bytes at the second placement).
    pub fixed: Vec<Finding>,
}

fn word(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// The window rule over `text` and `moved`: the same text as read at the first and at the
/// second placement of every [`Placed`] range. Panics if the lengths differ.
pub fn windows(text: &[u8], moved: &[u8], live: &Live) -> Windows {
    assert_eq!(text.len(), moved.len(), "the two readings of one text");
    let mut out = Windows::default();
    for at in 0..text.len().saturating_sub(7) {
        let (v, w) = (word(text, at), word(moved, at));
        if let Some(n) = live.hit(v) {
            out.found.push(Finding {
                at,
                what: format!("8 bytes {v:#x} = an address near {n}"),
            });
            continue;
        }
        // The mapped text lies inside the code range, so one value can hit both.
        let mut hits = live.placed.iter().filter(|p| p.at.contains(&v)).peekable();
        let Some(&first) = hits.peek() else {
            continue;
        };
        if let Some(p) = hits.find(|p| w == p.moved_value(v)) {
            out.found.push(Finding {
                at,
                what: format!(
                    "8 bytes {v:#x} = an address in {}; it moves with it (to {w:#x})",
                    p.name
                ),
            });
        } else {
            out.fixed.push(Finding {
                at,
                what: format!(
                    "8 bytes {v:#x} = an address in {} at this placement only ({w:#x} at the \
                     second)",
                    first.name
                ),
            });
        }
    }
    out
}

fn sext(v: u32, bits: u32) -> i64 {
    let s = 64 - bits;
    ((u64::from(v) << s) as i64) >> s
}

/// The AArch64 rules over the functions `fns` (text offset ranges) of `text`.
pub fn aarch64(text: &[u8], fns: &[Range<usize>], live: &Live) -> Vec<Finding> {
    let mut out = Vec::new();
    for f in fns {
        let mut regs = [None::<u64>; 32];
        let mut at = f.start;
        while at + 4 <= f.end {
            let w = u32::from_le_bytes(text[at..at + 4].try_into().unwrap());
            let rel = |imm: i64| -> Option<i64> { Some(at as i64 + imm) };
            let mut target = None;
            let mut what = "";
            if w & 0x9F00_0000 == 0x9000_0000 {
                out.push(Finding {
                    at,
                    what: format!("adrp {w:#010x}: a page-relative reference"),
                });
            } else if w & 0x9F00_0000 == 0x1000_0000 {
                let imm = ((w >> 29) & 3) | (((w >> 5) & 0x7FFFF) << 2);
                target = rel(sext(imm, 21));
                what = "adr";
            } else if w & 0x3B00_0000 == 0x1800_0000 {
                target = rel(sext((w >> 5) & 0x7FFFF, 19) * 4);
                what = "ldr literal";
            } else if w & 0xFC00_0000 == 0x9400_0000 {
                out.push(Finding {
                    at,
                    what: format!("bl {w:#010x}: a direct call"),
                });
            } else if w & 0xFC00_0000 == 0x1400_0000 {
                target = rel(sext(w & 0x03FF_FFFF, 26) * 4);
                what = "b";
            } else if w & 0xFF00_0010 == 0x5400_0000 {
                target = rel(sext((w >> 5) & 0x7FFFF, 19) * 4);
                what = "b.cond";
            } else if w & 0x7E00_0000 == 0x3400_0000 {
                target = rel(sext((w >> 5) & 0x7FFFF, 19) * 4);
                what = "cbz/cbnz";
            } else if w & 0x7E00_0000 == 0x3600_0000 {
                target = rel(sext((w >> 5) & 0x3FFF, 14) * 4);
                what = "tbz/tbnz";
            }
            if let Some(t) = target
                && !(f.start as i64..f.end as i64).contains(&t)
            {
                out.push(Finding {
                    at,
                    what: format!("{what} {w:#010x} leaves its function (to {t:#x})"),
                });
            }
            // movz / movn / movk, 64-bit: the constant a register holds.
            let (rd, hw, imm) = (
                (w & 31) as usize,
                (w >> 21) & 3,
                u64::from((w >> 5) & 0xFFFF),
            );
            let built = match w & 0xFF80_0000 {
                0xD280_0000 => Some(imm << (16 * hw)),
                0x9280_0000 => Some(!(imm << (16 * hw))),
                0xF280_0000 => regs[rd].map(|v| (v & !(0xFFFF << (16 * hw))) | (imm << (16 * hw))),
                _ => {
                    // Any other write to rd ends its constant (a conservative reset: the
                    // destination field is bits 0-4 in every form this emitter uses).
                    regs[rd] = None;
                    None
                }
            };
            if let Some(v) = built {
                regs[rd] = Some(v);
                // Fixed addresses only: a constant of the instruction bits that equals a
                // placed address has one reading here (the window rule's two readings decide).
                if let Some(n) = live.hit(v) {
                    out.push(Finding {
                        at,
                        what: format!("mov x{rd} builds {v:#x} = an address near {n}"),
                    });
                }
            }
            at += 4;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::Arch;
    use crate::unit::build::build;
    use crate::unit::checks::unit_a;
    use crate::unit::format::UnitView;

    fn words(w: &[u32]) -> Vec<u8> {
        w.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    fn live() -> Live {
        let mut l = Live::default();
        l.point("cx", 0x0000_7f12_3456_0000);
        l
    }

    /// The process code range at the 4 GiB slot `base` with its first chunk (the unit text,
    /// `len` bytes) at the slot itself, as LOAD places it, and a second placement 4 GiB
    /// higher.
    fn placed_at(base: u64, len: u64) -> Live {
        let (b, m) = (base as usize, base as usize + (4 << 30));
        let mut l = live();
        l.placed("the unit text", b..b + len as usize, m).unwrap();
        l.placed("the code range", b..b + (128 << 20), m).unwrap();
        l
    }

    /// Negative controls: each rule finds the defect it names.
    #[test]
    fn the_scan_finds_planted_relocations() {
        let l = live();
        // A clean function: movz x16, #1 ; b .+4 ; ret.
        let clean = words(&[0xD280_0030, 0x1400_0001, 0xD65F_03C0]);
        assert!(aarch64(&clean, std::slice::from_ref(&(0..12)), &l).is_empty());
        assert_eq!(windows(&clean, &clean, &l), Windows::default());
        // The address in 8 bytes of text.
        let mut t = clean.clone();
        t.extend_from_slice(&0x0000_7f12_3456_0010u64.to_le_bytes());
        assert_eq!(windows(&t, &t, &l).found.len(), 1);
        // movz/movk building it: movz x9, #0x0010 ; movk x9, #0x3456, lsl 16 ;
        // movk x9, #0x7f12, lsl 32.
        let m = words(&[0xD280_0209, 0xF2A6_8AC9, 0xF2CF_E249]);
        let f = aarch64(&m, std::slice::from_ref(&(0..12)), &l);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].what.contains("near cx"), "{f:?}");
        // adrp, bl, and a branch out of the function.
        let bad = words(&[0x9000_0010, 0x9400_0000, 0x1400_0010]);
        let f = aarch64(&bad, std::slice::from_ref(&(0..12)), &l);
        assert_eq!(f.len(), 3, "{f:?}");
        // b.cond and cbz that leave; an in-range cbz does not.
        let bc = words(&[0x5400_0400, 0xB400_0400, 0xB4FF_FFE0]);
        assert_eq!(aarch64(&bc, std::slice::from_ref(&(0..12)), &l).len(), 2);
    }

    /// M6-ORCH-11 (CI 41420942032): fixed text bytes that equal the mapped base are no
    /// finding. The 8 bytes at offset 872 of the x86-64 unit A text were
    /// `00 00 00 00 e9 32 00 00` (a 32-bit zero, then `jmp rel32` 0x32), and the code range
    /// and the text started at `0x32e9_0000_0000`.
    #[test]
    fn fixed_bytes_equal_to_the_mapped_base_are_no_finding() {
        let mut t = words(&[0xD65F_03C0]);
        let at = t.len();
        t.extend_from_slice(&[0, 0, 0, 0, 0xe9, 0x32, 0, 0, 0, 0xc3]);
        let l = placed_at(0x32e9_0000_0000, t.len() as u64);
        // The same bytes at both placements (LOAD copies them unchanged).
        let w = windows(&t, &t, &l);
        assert!(w.found.is_empty(), "{w:?}");
        assert_eq!(w.fixed.len(), 1, "{w:?}");
        assert_eq!(w.fixed[0].at, at);
        assert!(w.fixed[0].what.contains("0x32e900000000"), "{w:?}");
        // Elsewhere the same bytes hit nothing.
        assert_eq!(
            windows(&t, &t, &placed_at(0x1234_0000_0000, t.len() as u64)),
            Windows::default()
        );
        // The real x86-64 unit A text, with the code range placed at each 4 GiB slot that
        // one of its windows names: 0 findings, whatever the slot.
        let bytes = build(&unit_a(), Arch::X86_64).expect("unit A");
        let text = UnitView::parse(&bytes).expect("parse").text;
        let mut slots = 0;
        for at in 0..text.len() - 7 {
            let v = u64::from_le_bytes(text[at..at + 8].try_into().unwrap());
            if v & 0xFFFF_FFFF == 0 && (1..1 << 15).contains(&(v >> 32)) {
                slots += 1;
                let w = windows(text, text, &placed_at(v, text.len() as u64));
                assert!(w.found.is_empty(), "slot {v:#x}: {w:?}");
                assert!(w.fixed.iter().any(|f| f.at == at), "slot {v:#x}: {w:?}");
            }
        }
        eprintln!("x86-64 unit A: {slots} windows name a 4 GiB slot");
    }

    /// A real address is still found: one near a fixed address at any placement, and one in
    /// a placed range that moves with it.
    #[test]
    fn a_real_address_is_found() {
        let base = 0x32e9_0000_0000u64;
        let mut t = words(&[0xD65F_03C0, 0xD503_201F]);
        let l = placed_at(base, 0x4000);
        // The Cx address (a fixed point), the same at both placements.
        t.extend_from_slice(&0x0000_7f12_3456_0010u64.to_le_bytes());
        let w = windows(&t, &t, &l);
        assert_eq!(w.found.len(), 1, "{w:?}");
        assert!(w.found[0].what.contains("near cx"), "{w:?}");
        // A reference to the text's offset 0x40: base + 0x40 at the first placement, and the
        // second placement's start + 0x40 at the second.
        let at = t.len();
        let mut first = t.clone();
        first.extend_from_slice(&(base + 0x40).to_le_bytes());
        let mut moved = t.clone();
        moved.extend_from_slice(&(base + (4 << 30) + 0x40).to_le_bytes());
        let w = windows(&first, &moved, &l);
        assert_eq!(w.found.len(), 2, "{w:?}");
        assert_eq!(w.found[1].at, at);
        assert!(w.found[1].what.contains("moves with it"), "{w:?}");
        assert!(w.fixed.is_empty(), "{w:?}");
        // The same value at both readings is a fixed value, not a reference.
        let w = windows(&first, &first, &l);
        assert_eq!((w.found.len(), w.fixed.len()), (1, 1), "{w:?}");
        // Two placements at one address cannot show a move.
        assert!(Live::default().placed("x", 0x1000..0x2000, 0x1000).is_err());
    }
}
