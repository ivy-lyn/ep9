//! The unit emitter: the 11 Spike B opcodes as unit text, behind the frozen
//! [`crate::emit::Emitter`] trait. The code is placement (b) code (`crate::jit_b`: the T0 handler
//! ABI, the `Return` tail jump through `cx.table`), plus the unit-data register (H35-Q9).
//!
//! ```text
//! entry (cx, pc, fp, acc)       x0-x3 (AArch64)          rdi, rsi, rdx, rcx (x86-64, sysv64)
//!   save the unit-data register  str x27, [sp, #32]       push r14
//!   x27 = cx.unit_data[slot]     ldr x27, [x28, #ud]      mov r14, [r15 + ud]
//!                                ldr x27, [x27, #8*slot]  mov r14, [r14 + 8*slot]
//!   call counter += 1            [x27 + counter]          add qword [r14 + counter], 1
//!   a table call                 x16 = [x27 + ext 0]      rax = [r14 + ext 0]     (link cell)
//!   a call site (IC cell)        [x27 + ic] = callee when the cell is still undefined
//!   every exit restores x27 / r14 before its tail jump (b code is entered by a tail jump from a
//!   Rust handler, and that handler's caller expects every callee-saved register back)
//! ```
//!
//! The RW copy of a unit (one per isolate, [`RwLayout`]), in 8-byte words from the unit-data
//! register:
//!
//! | Words | What | GC |
//! |---|---|---|
//! | `ext` | external references (word 0: the `RuntimeTable`), resolved at load | none (raw addresses) |
//! | `links` | link cells: Values by stable index (an intrinsic) or unit constants, resolved at load | strong root range |
//! | `ics` | IC cells, one per call site: the first callee, filled at first execution | weak root range |
//! | `counters` | call counters, one per function | none (raw integers) |
//!
//! The unit's slot (`RwLayout::slot`, 0..[`SLOTS`]) is fixed when the unit is built, so the
//! entry sequence holds no address and needs no load-time patch. Two different units with one
//! slot cannot both map in one process (the second one falls back to its bytecode).

use crate::emit::{Arch, Emitter, FunctionCode, FunctionInput, Refusal};
use crate::jit::{Op, Plan, plan, slot_bytes};

/// Unit slots per isolate: the size of the unit-data directory `Cx::unit_data` points at.
pub const SLOTS: usize = 64;

/// The external-reference word that holds the `RuntimeTable` address.
pub const EXT_RUNTIME_TABLE: u32 = 0;

/// The RW layout of one unit: its slot and the word counts of each region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RwLayout {
    /// The unit's slot in the unit-data directory.
    pub slot: u32,
    /// External references (raw addresses).
    pub ext: u32,
    /// Link cells (Values; a strong root range).
    pub links: u32,
    /// IC cells (Values; a weak root range).
    pub ics: u32,
    /// Call counters (raw integers).
    pub counters: u32,
}

impl RwLayout {
    /// The size of the RW copy in words.
    pub fn words(&self) -> usize {
        (self.ext + self.links + self.ics + self.counters) as usize
    }

    /// The word index of the first link cell.
    pub fn link_base(&self) -> usize {
        self.ext as usize
    }

    /// The word index of the first IC cell.
    pub fn ic_base(&self) -> usize {
        (self.ext + self.links) as usize
    }

    /// The word index of the first counter.
    pub fn counter_base(&self) -> usize {
        (self.ext + self.links + self.ics) as usize
    }

    /// The byte offset from the unit-data register of external reference `i`.
    pub fn ext_off(&self, i: u32) -> usize {
        8 * i as usize
    }

    /// The byte offset of IC cell `i`.
    pub fn ic_off(&self, i: u32) -> usize {
        8 * (self.ic_base() + i as usize)
    }

    /// The byte offset of counter `i`.
    pub fn counter_off(&self, i: u32) -> usize {
        8 * (self.counter_base() + i as usize)
    }

    /// The byte offset of this unit's word in the unit-data directory.
    pub fn slot_off(&self) -> usize {
        8 * self.slot as usize
    }
}

/// One call site of a function: its bytecode offset, the frame slot of its callee register,
/// and its IC cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Site {
    pub offset: u32,
    pub callee: i32,
    pub ic: u32,
}

/// What a code generator needs about the function besides its plan.
#[derive(Clone, Debug)]
pub(crate) struct FnCtx {
    pub layout: RwLayout,
    /// The function's call counter.
    pub counter: u32,
    /// Its call sites, in offset order.
    pub sites: Vec<Site>,
}

impl FnCtx {
    pub fn site(&self, offset: u32) -> Result<Site, Refusal> {
        self.sites
            .iter()
            .copied()
            .find(|s| s.offset == offset)
            .ok_or_else(|| Refusal::Limit(format!("no IC cell for the call at {offset}")))
    }
}

/// The plan of `input` and its context: counter `counter`, IC cells from `first_ic` on, one
/// per call site in offset order.
fn fn_ctx(
    input: &FunctionInput<'_>,
    layout: RwLayout,
    counter: u32,
    first_ic: u32,
) -> Result<(Plan, FnCtx), Refusal> {
    let p = plan(input)?;
    if counter >= layout.counters {
        return Err(Refusal::Limit(format!(
            "{} counters in the layout, function {} needs one more",
            layout.counters, input.name
        )));
    }
    let mut sites = Vec::new();
    for s in p
        .steps
        .iter()
        .filter(|s| s.op == Op::CallUndefinedReceiver1)
    {
        let insn =
            ep9_bytecode::decode_at(input.bytecode, s.offset).map_err(|e| Refusal::Decode {
                offset: s.offset,
                detail: format!("{e:?}"),
            })?;
        let ic = first_ic + sites.len() as u32;
        if ic >= layout.ics {
            return Err(Refusal::Limit(format!(
                "{} IC cells in the layout, {} needs more",
                layout.ics, input.name
            )));
        }
        sites.push(Site {
            offset: s.offset,
            callee: slot_bytes(insn.register(0).to_operand()),
            ic,
        });
    }
    Ok((
        p,
        FnCtx {
            layout,
            counter,
            sites,
        },
    ))
}

/// The RW words the emitter gave one function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assigned {
    /// `FunctionInput::key`.
    pub key: usize,
    /// Its call counter.
    pub counter: u32,
    /// Its first IC cell and the number of its call sites.
    pub ic_base: u32,
    pub ic_count: u32,
}

/// The number of call sites (`CallUndefinedReceiver1`) the plan of `input` has, or why the
/// emitter refuses the function.
pub fn call_sites(input: &FunctionInput<'_>) -> Result<u32, Refusal> {
    let p = plan(input)?;
    Ok(p.steps
        .iter()
        .filter(|s| s.op == Op::CallUndefinedReceiver1)
        .count() as u32)
}

/// The unit emitter for one architecture and one unit layout. It gives each function the next
/// counter and the next IC cells, in emit order ([`UnitEmitter::assigned`]).
#[derive(Clone, Debug)]
pub struct UnitEmitter {
    arch: Arch,
    layout: RwLayout,
    next_ic: u32,
    assigned: Vec<Assigned>,
}

impl UnitEmitter {
    pub fn new(arch: Arch, layout: RwLayout) -> Result<UnitEmitter, Refusal> {
        if layout.slot as usize >= SLOTS {
            return Err(Refusal::Limit(format!(
                "unit slot {} >= {SLOTS}",
                layout.slot
            )));
        }
        if layout.ext < EXT_RUNTIME_TABLE + 1 {
            return Err(Refusal::Limit("no runtime-table word".into()));
        }
        Ok(UnitEmitter {
            arch,
            layout,
            next_ic: 0,
            assigned: Vec::new(),
        })
    }

    pub fn layout(&self) -> RwLayout {
        self.layout
    }

    /// The words each emitted function got, in emit order.
    pub fn assigned(&self) -> &[Assigned] {
        &self.assigned
    }

    /// The unwind self-test probe in unit text (`ep9_platform::unwind::ProbeFn` on unix:
    /// `probe(x, cb) = cb(x + 1) + x` with a frame record; x86-64: System V on every OS).
    pub fn probe(&self) -> Result<FunctionCode, Refusal> {
        let (code, unwind) = match self.arch {
            Arch::Aarch64 => super::a64::probe()?,
            Arch::X86_64 => super::x64::probe()?,
        };
        Ok(extra(self.arch, "<probe>", code, unwind))
    }

    /// The cell reader in unit text: `cell(cx, i) = word i of the RW copy` that the unit-data
    /// register reaches from `cx` (the entry sequence of every unit function). `extern "C"`
    /// (AArch64), `extern "sysv64"` (x86-64).
    pub fn cell_reader(&self) -> Result<FunctionCode, Refusal> {
        let (code, unwind) = match self.arch {
            Arch::Aarch64 => super::a64::cell_reader(&self.layout)?,
            Arch::X86_64 => super::x64::cell_reader(&self.layout)?,
        };
        Ok(extra(self.arch, "<cell>", code, unwind))
    }
}

fn extra(arch: Arch, name: &str, code: Vec<u8>, unwind: crate::emit::Unwind) -> FunctionCode {
    FunctionCode {
        name: name.to_string(),
        arch,
        key: usize::MAX,
        code,
        unwind,
        ops: Vec::new(),
        native_offsets: Vec::new(),
    }
}

impl Emitter for UnitEmitter {
    fn arch(&self) -> Arch {
        self.arch
    }

    fn emit(&mut self, input: &FunctionInput<'_>) -> Result<FunctionCode, Refusal> {
        let counter = self.assigned.len() as u32;
        let (p, f) = fn_ctx(input, self.layout, counter, self.next_ic)?;
        let (code, unwind, native_offsets) = match self.arch {
            Arch::Aarch64 => super::a64::emit(&p, &f)?,
            Arch::X86_64 => super::x64::emit(&p, &f)?,
        };
        self.assigned.push(Assigned {
            key: input.key,
            counter,
            ic_base: self.next_ic,
            ic_count: f.sites.len() as u32,
        });
        self.next_ic += f.sites.len() as u32;
        Ok(FunctionCode {
            name: input.name.to_string(),
            arch: self.arch,
            key: input.key,
            code,
            unwind,
            ops: p.steps.iter().map(|s| (s.offset, s.opcode)).collect(),
            native_offsets,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::compile;
    use crate::structural::{LOOP_DEFS, MIXED_DEFS};
    use crate::unit::build::function_input;
    use crate::unit::{scan, x64};

    /// The JS functions of units A and B, emitted for `arch` into one layout.
    fn emitted(arch: Arch) -> Vec<FunctionCode> {
        let unit = compile(&format!("{LOOP_DEFS}{MIXED_DEFS}")).unwrap();
        let names = ["id", "drive", "jcall", "twice", "inc", "lt"];
        let inputs: Vec<_> = names
            .iter()
            .map(|n| function_input(&unit, n).unwrap())
            .collect();
        let ics = inputs.iter().map(|i| call_sites(i).unwrap()).sum();
        let layout = RwLayout {
            slot: 5,
            ext: 1,
            links: 2,
            ics,
            counters: inputs.len() as u32,
        };
        let mut em = UnitEmitter::new(arch, layout).unwrap();
        let out = inputs.iter().map(|i| em.emit(i).unwrap()).collect();
        assert_eq!(em.assigned().len(), names.len());
        assert_eq!(
            em.assigned().iter().map(|a| a.ic_count).sum::<u32>(),
            ics,
            "one IC cell per call site"
        );
        out
    }

    fn words(code: &[u8]) -> Vec<u32> {
        code.as_chunks::<4>()
            .0
            .iter()
            .map(|w| u32::from_le_bytes(*w))
            .collect()
    }

    /// AArch64: the 48-byte frame saves x27; every exit is `br x16` after the restore of x27,
    /// x26, x28, x29 and x30; no `ret`; the relocation rules find nothing.
    #[test]
    fn the_aarch64_unit_frame_saves_and_restores_x27_on_every_exit() {
        const PROLOGUE: [u32; 4] = [0xA9BD_7BFD, 0x9100_03FD, 0xA901_73FA, 0xF900_13FB];
        const EPILOGUE: [u32; 4] = [0xF940_13FB, 0xA941_73FA, 0xA8C3_7BFD, 0xD61F_0200];
        for f in emitted(Arch::Aarch64) {
            let w = words(&f.code);
            assert_eq!(w[..4], PROLOGUE, "{}", f.name);
            let mut exits = 0;
            for (i, &x) in w.iter().enumerate() {
                assert_ne!(x, 0xD65F_03C0, "{}: ret at {}", f.name, 4 * i);
                if x & 0xFFFF_FC1F == 0xD61F_0000 {
                    assert!(
                        i >= 3 && w[i - 3..=i] == EPILOGUE,
                        "{}: exit at {}",
                        f.name,
                        4 * i
                    );
                    exits += 1;
                }
            }
            assert!(exits >= 1, "{}", f.name);
            let found = scan::aarch64(
                &f.code,
                std::slice::from_ref(&(0..f.code.len())),
                &scan::Live::default(),
            );
            assert!(found.is_empty(), "{}: {found:?}", f.name);
        }
    }

    /// x86-64: the 5-push frame saves r14; every exit jump follows the restore of rbx, r14,
    /// r13, r15 and rbp.
    #[test]
    fn the_x86_64_unit_frame_saves_and_restores_r14_on_every_exit() {
        const PROLOGUE: [u8; 12] = [
            0x55, 0x41, 0x57, 0x41, 0x55, 0x41, 0x56, 0x53, 0x48, 0x8D, 0x6C, 0x24,
        ];
        const EPILOGUE: [u8; 11] = [
            0x48, 0x8D, 0x65, 0xE0, 0x5B, 0x41, 0x5E, 0x41, 0x5D, 0x41, 0x5F,
        ];
        for f in emitted(Arch::X86_64) {
            assert_eq!(f.code[..12], PROLOGUE, "{}", f.name);
            assert_eq!(f.code[12], 0x20, "{}: lea rbp, [rsp + 32]", f.name);
            let mut exits = 0;
            for at in 0..f.code.len().saturating_sub(3) {
                if f.code[at..at + 4] == x64::EXIT_JMP {
                    assert!(at >= 12, "{}", f.name);
                    assert_eq!(
                        f.code[at - 12..at - 1],
                        EPILOGUE,
                        "{}: exit at {at}",
                        f.name
                    );
                    assert_eq!(f.code[at - 1], 0x5D, "{}: pop rbp at {}", f.name, at - 1);
                    exits += 1;
                }
            }
            assert!(exits >= 1, "{}", f.name);
        }
    }

    /// Two emits of one input give the same bytes (the text depends on the input and the
    /// layout only).
    #[test]
    fn two_emits_give_the_same_text() {
        for arch in [Arch::Aarch64, Arch::X86_64] {
            assert_eq!(
                emitted(arch).iter().map(|f| &f.code).collect::<Vec<_>>(),
                emitted(arch).iter().map(|f| &f.code).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_slot_out_of_range_is_refused() {
        let l = RwLayout {
            slot: SLOTS as u32,
            ext: 1,
            links: 0,
            ics: 0,
            counters: 1,
        };
        assert!(UnitEmitter::new(Arch::Aarch64, l).is_err());
        assert!(
            UnitEmitter::new(
                Arch::X86_64,
                RwLayout {
                    ext: 0,
                    slot: 0,
                    ..l
                }
            )
            .is_err()
        );
    }
}
