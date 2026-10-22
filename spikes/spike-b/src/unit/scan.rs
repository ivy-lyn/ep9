//! The relocation scan of unit text ("a text scan finds 0 absolute relocations").
//!
//! | Rule | AArch64 | x86-64 |
//! |---|---|---|
//! | no 8-byte window (any alignment) is a live address of this process (within 64 KiB of one: the Cx, the runtime table and its functions, FunctionInfos, bytecode, heap cells, the RW copy, the directory, the text itself) or lies in the code range | yes | yes |
//! | no constant that `movz`/`movn`/`movk` build in a register is such an address | yes | - |
//! | no `adrp` (a page-relative reference) | yes | - |
//! | every pc-relative form (`b`, `b.cond`, `cbz`/`cbnz`, `tbz`/`tbnz`, `adr`, `ldr` literal) stays inside its own function; no direct call (`bl`) | yes | - |
//!
//! The x86-64 code generator writes branches only through dynasm labels inside one function
//! (`jit_b::x64` rule) and has no decoder here; its text gets the window rule and the
//! file-equals-mapped rule of the load check.

use std::ops::Range;

/// One finding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// Text offset.
    pub at: usize,
    pub what: String,
}

/// Addresses the scan looks for: exact addresses (with a 64 KiB margin each way) and ranges.
#[derive(Clone, Debug, Default)]
pub struct Live {
    pub points: Vec<(String, u64)>,
    pub ranges: Vec<(String, Range<u64>)>,
}

/// The margin around each exact address.
pub const MARGIN: u64 = 64 << 10;

impl Live {
    pub fn point(&mut self, name: &str, addr: usize) -> &mut Self {
        self.points.push((name.to_string(), addr as u64));
        self
    }

    pub fn range(&mut self, name: &str, r: Range<usize>) -> &mut Self {
        self.ranges
            .push((name.to_string(), r.start as u64..r.end as u64));
        self
    }

    /// The name of the live address `v` is, or is near.
    pub fn hit(&self, v: u64) -> Option<&str> {
        if v < 1 << 16 {
            return None;
        }
        for (n, p) in &self.points {
            if v.abs_diff(*p) < MARGIN {
                return Some(n);
            }
        }
        self.ranges
            .iter()
            .find(|(_, r)| r.contains(&v))
            .map(|(n, _)| n.as_str())
    }
}

/// The window rule over `text`.
pub fn windows(text: &[u8], live: &Live) -> Vec<Finding> {
    let mut out = Vec::new();
    for at in 0..text.len().saturating_sub(7) {
        let v = u64::from_le_bytes(text[at..at + 8].try_into().unwrap());
        if let Some(n) = live.hit(v) {
            out.push(Finding {
                at,
                what: format!("8 bytes {v:#x} = an address near {n}"),
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

    fn words(w: &[u32]) -> Vec<u8> {
        w.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    fn live() -> Live {
        let mut l = Live::default();
        l.point("cx", 0x0000_7f12_3456_0000);
        l
    }

    /// Negative controls: each rule finds the defect it names.
    #[test]
    fn the_scan_finds_planted_relocations() {
        let l = live();
        // A clean function: movz x16, #1 ; b .+4 ; ret.
        let clean = words(&[0xD280_0030, 0x1400_0001, 0xD65F_03C0]);
        assert!(aarch64(&clean, std::slice::from_ref(&(0..12)), &l).is_empty());
        assert!(windows(&clean, &l).is_empty());
        // The address in 8 bytes of text.
        let mut t = clean.clone();
        t.extend_from_slice(&0x0000_7f12_3456_0010u64.to_le_bytes());
        assert_eq!(windows(&t, &l).len(), 1);
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
}
