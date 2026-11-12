//! The `.o` form of a unit (`ep9_unit::obj`, H35-Q3; route MC-w34-5; M8-P3 A2-OBJ).
//!
//! ```text
//! TestUnit (text: one small function, unwind: form 1 of the target) --encode--> unit bytes
//!   --write_object--> object bytes --read back here (ELF, Mach-O, COFF)--> sections, relocations
//!   host only: object --rustc (a small program)--> binary: the record names the container
//!   and the linked text; the text runs; a panic unwinds through it (the linker's unwind tables)
//! ```
//!
//! | Test | What it shows |
//! |---|---|
//! | `each_target_object_has_its_sections_and_no_text_relocation` | 5 targets: the section names of `section_names`, the text and the container unchanged, zero text relocations (H35 S5), 2 record relocations, one unwind relocation per FDE (COFF: 3 per entry), the record symbol |
//! | `the_writer_refuses_what_has_no_object_form` | a bad symbol, no text, Windows on AArch64, a bad unwind section |
//! | `check_linked_needs_the_same_text` | `check_linked` is `check` when the linked bytes are the text, else Bytecode |
//! | `the_object_links_and_runs_on_the_host` | the 4 CI targets (host only): the static linker takes the object; the text runs in place; a panic unwinds through the unit frame |
#![cfg(feature = "aot")]

mod common;

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{TestUnit, filler, header_for};
use ep9_unit::obj::{AppleOs, ObjError, ObjFormat, section_names, write_object, write_object_for};
use ep9_unit::{HostInfo, Outcome, SectionKind, check, check_linked, target_abi, target_arch};

// ---------------------------------------------------------------------------------------------
// A unit with one function: `f(cb)` calls `cb()` with a frame and returns.

/// AArch64: stp x29, x30, [sp, #-16]!; mov x29, sp; blr x0; ldp x29, x30, [sp], #16; ret.
fn a64_text() -> Vec<u8> {
    [
        0xa9bf_7bfdu32,
        0x9100_03fd,
        0xd63f_0000,
        0xa8c1_7bfd,
        0xd65f_03c0,
    ]
    .iter()
    .flat_map(|w| w.to_le_bytes())
    .collect()
}

/// x86-64 System V: push rbp; mov rbp, rsp; call rdi; pop rbp; ret.
const X64_SYSV_TEXT: [u8; 8] = [0x55, 0x48, 0x89, 0xe5, 0xff, 0xd7, 0x5d, 0xc3];

/// x86-64 Windows: push rbp; sub rsp, 32; call rcx; add rsp, 32; pop rbp; ret (13 bytes), 3
/// bytes of int3, then the UNWIND_INFO at 16: version 1, prolog 5 bytes, 2 codes (at 5:
/// UWOP_ALLOC_SMALL 32; at 1: UWOP_PUSH_NONVOL rbp).
const X64_WIN_TEXT: [u8; 24] = [
    0x55, 0x48, 0x83, 0xec, 0x20, 0xff, 0xd1, 0x48, 0x83, 0xc4, 0x20, 0x5d, 0xc3, 0xcc, 0xcc, 0xcc,
    0x01, 0x05, 0x02, 0x00, 0x05, 0x32, 0x01, 0x50,
];

/// One unix unwind record of form 1, padded to 8 bytes with DW_CFA_nop.
fn record(out: &mut Vec<u8>, body: &[u8]) {
    let len = (4 + body.len()).next_multiple_of(8) - 4;
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.extend_from_slice(body);
    out.resize(out.len() + len - body.len(), 0);
}

/// The unix unwind section of form 1 (module doc of `ep9_vm` `unit_loader.rs`): one CIE, one
/// FDE over `0..len` of the text, the zero terminator.
fn eh_unwind(arch: u8, len: usize) -> Vec<u8> {
    let (cie_tail, fde_insns): (&[u8], &[u8]) = if arch == target_arch::AARCH64 {
        // code align 4, data align -8, return address x30; CFA = sp.
        // After stp: CFA = sp + 16, x30 at CFA - 8, x29 at CFA - 16.
        (
            &[4, 0x78, 30, 1, 0, 0x0c, 31, 0],
            &[0, 0x41, 0x0e, 16, 0x9e, 1, 0x9d, 2],
        )
    } else {
        // code align 1, data align -8, return address 16; CFA = rsp + 8, ra at CFA - 8.
        // After push rbp: CFA = rsp + 16, rbp at CFA - 16; after mov: CFA = rbp + 16.
        (
            &[1, 0x78, 16, 1, 0, 0x0c, 7, 8, 0x90, 1],
            &[0, 0x41, 0x0e, 16, 0x86, 2, 0x43, 0x0d, 6],
        )
    };
    let mut out = Vec::new();
    let mut cie = vec![0, 0, 0, 0, 1, b'z', b'R', 0];
    cie.extend_from_slice(cie_tail);
    record(&mut out, &cie);
    let fde_at = out.len();
    let mut fde = ((fde_at + 4) as u32).to_le_bytes().to_vec();
    fde.extend_from_slice(&0u64.to_le_bytes());
    fde.extend_from_slice(&(len as u64).to_le_bytes());
    fde.extend_from_slice(fde_insns);
    record(&mut out, &fde);
    out.extend_from_slice(&[0; 4]);
    out
}

/// The text and the unwind section of a target.
fn code(arch: u8, abi: u8) -> (Vec<u8>, Vec<u8>) {
    if abi == target_abi::WINDOWS {
        let mut pdata = Vec::new();
        for v in [0u32, 13, 16] {
            pdata.extend_from_slice(&v.to_le_bytes());
        }
        return (X64_WIN_TEXT.to_vec(), pdata);
    }
    let text = if arch == target_arch::AARCH64 {
        a64_text()
    } else {
        X64_SYSV_TEXT.to_vec()
    };
    let unwind = eh_unwind(arch, text.len());
    (text, unwind)
}

/// A unit for `host` with the given target, its text and unwind section.
fn unit_for(host: &HostInfo, arch: u8, abi: u8) -> (Vec<u8>, Vec<u8>) {
    let (text, unwind) = code(arch, abi);
    let mut header = header_for(host, 0);
    header.target_arch = arch;
    header.target_abi = abi;
    let unit = TestUnit {
        header,
        sections: vec![
            (SectionKind::Text, text.clone()),
            (SectionKind::Bytecode, filler(21, 37)),
            (SectionKind::Source, b"f(cb);\n".to_vec()),
            (SectionKind::Unwind, unwind),
        ],
        extensions: Vec::new(),
    };
    (unit.encode(), text)
}

const TARGETS: [(u8, u8, ObjFormat); 5] = [
    (target_arch::X86_64, target_abi::LINUX, ObjFormat::Elf),
    (target_arch::AARCH64, target_abi::LINUX, ObjFormat::Elf),
    (target_arch::X86_64, target_abi::APPLE, ObjFormat::MachO),
    (target_arch::AARCH64, target_abi::APPLE, ObjFormat::MachO),
    (target_arch::X86_64, target_abi::WINDOWS, ObjFormat::Coff),
];

// ---------------------------------------------------------------------------------------------
// Reading the objects back (each format by its own spec, independent of the writer's code).

/// One section of an object: name, bytes, relocations (offset, type, symbol name).
#[derive(Debug)]
struct Sec {
    name: String,
    data: Vec<u8>,
    relocs: Vec<(u64, u32, String)>,
}

/// An object read back: its sections and its global symbols (name, section name).
#[derive(Debug)]
struct Obj {
    sections: Vec<Sec>,
    globals: Vec<(String, String)>,
}

impl Obj {
    fn sec(&self, name: &str) -> &Sec {
        self.sections
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no section {name} in {:?}", self.names()))
    }
    fn names(&self) -> Vec<&str> {
        self.sections.iter().map(|s| s.name.as_str()).collect()
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn cstr(b: &[u8], at: usize) -> String {
    let end = b[at..].iter().position(|&c| c == 0).unwrap() + at;
    String::from_utf8(b[at..end].to_vec()).unwrap()
}
fn fixed(b: &[u8]) -> String {
    cstr(&[b, &[0]].concat(), 0)
}

fn read_elf(b: &[u8]) -> Obj {
    assert_eq!(&b[..6], &[0x7f, b'E', b'L', b'F', 2, 1]);
    assert_eq!(u16_at(b, 16), 1, "ET_REL");
    let shoff = u64_at(b, 40) as usize;
    let (shentsize, shnum, shstrndx) = (
        u16_at(b, 58) as usize,
        u16_at(b, 60) as usize,
        u16_at(b, 62) as usize,
    );
    let sh = |i: usize| &b[shoff + i * shentsize..shoff + (i + 1) * shentsize];
    let data = |i: usize| {
        let h = sh(i);
        let (off, size) = (u64_at(h, 24) as usize, u64_at(h, 32) as usize);
        &b[off..off + size]
    };
    let shstr = data(shstrndx);
    let name = |i: usize| cstr(shstr, u32_at(sh(i), 0) as usize);
    let symtab = (1..shnum).find(|&i| u32_at(sh(i), 4) == 2).unwrap();
    let strtab = data(u32_at(sh(symtab), 40) as usize);
    let syms: Vec<(String, u8, u16)> = data(symtab)
        .as_chunks::<24>()
        .0
        .iter()
        .map(|s| (cstr(strtab, u32_at(s, 0) as usize), s[4], u16_at(s, 6)))
        .collect();
    let mut sections: Vec<Sec> = (0..shnum)
        .map(|i| Sec {
            name: name(i),
            data: data(i).to_vec(),
            relocs: Vec::new(),
        })
        .collect();
    for i in 1..shnum {
        if u32_at(sh(i), 4) == 4 {
            let target = u32_at(sh(i), 44) as usize;
            for r in data(i).as_chunks::<24>().0 {
                let info = u64_at(r, 8);
                let (sym, ty) = ((info >> 32) as usize, info as u32);
                let (n, st_info, shndx) = &syms[sym];
                let n = if st_info & 0xf == 3 {
                    name(*shndx as usize)
                } else {
                    n.clone()
                };
                sections[target].relocs.push((u64_at(r, 0), ty, n));
            }
        }
    }
    let globals = syms
        .iter()
        .filter(|s| s.1 >> 4 == 1)
        .map(|s| (s.0.clone(), name(s.2 as usize)))
        .collect();
    Obj { sections, globals }
}

fn read_macho(b: &[u8]) -> Obj {
    assert_eq!(u32_at(b, 0), 0xfeed_facf);
    assert_eq!(u32_at(b, 12), 1, "MH_OBJECT");
    let ncmds = u32_at(b, 16) as usize;
    let mut at = 32;
    let mut raw = Vec::new(); // (name, data, reloff, nreloc, addr)
    let (mut symoff, mut nsyms, mut stroff) = (0, 0, 0);
    for _ in 0..ncmds {
        let (cmd, size) = (u32_at(b, at), u32_at(b, at + 4) as usize);
        if cmd == 0x19 {
            for s in 0..u32_at(b, at + 64) as usize {
                let h = &b[at + 72 + 80 * s..at + 152 + 80 * s];
                let name = format!("{},{}", fixed(&h[16..32]), fixed(&h[..16]));
                let (addr, size, off) = (u64_at(h, 32), u64_at(h, 40) as usize, u32_at(h, 48));
                let data = b[off as usize..off as usize + size].to_vec();
                raw.push((
                    name,
                    data,
                    u32_at(h, 56) as usize,
                    u32_at(h, 60) as usize,
                    addr,
                ));
            }
        } else if cmd == 2 {
            symoff = u32_at(b, at + 8) as usize;
            nsyms = u32_at(b, at + 12) as usize;
            stroff = u32_at(b, at + 16) as usize;
        }
        at += size;
    }
    let syms: Vec<(String, u8, u8)> = (0..nsyms)
        .map(|i| {
            let s = &b[symoff + 16 * i..symoff + 16 * (i + 1)];
            (cstr(b, stroff + u32_at(s, 0) as usize), s[4], s[5])
        })
        .collect();
    let sections = raw
        .iter()
        .map(|(name, data, reloff, nreloc, _)| Sec {
            name: name.clone(),
            data: data.clone(),
            relocs: (0..*nreloc)
                .map(|i| {
                    let (addr, w) = (u32_at(b, reloff + 8 * i), u32_at(b, reloff + 8 * i + 4));
                    assert_ne!(w & (1 << 27), 0, "an extern relocation");
                    let sym = &syms[(w & 0xff_ffff) as usize];
                    (u64::from(addr), w >> 28, raw[sym.2 as usize - 1].0.clone())
                })
                .collect(),
        })
        .collect();
    let globals = syms
        .iter()
        .filter(|s| s.1 & 1 != 0)
        .map(|s| (s.0.clone(), raw[s.2 as usize - 1].0.clone()))
        .collect();
    Obj { sections, globals }
}

/// The `LC_BUILD_VERSION` of a Mach-O object: (platform, minos).
fn macho_build_version(b: &[u8]) -> Option<(u32, u32)> {
    let mut at = 32;
    for _ in 0..u32_at(b, 16) {
        if u32_at(b, at) == 0x32 {
            return Some((u32_at(b, at + 8), u32_at(b, at + 12)));
        }
        at += u32_at(b, at + 4) as usize;
    }
    None
}

fn read_coff(b: &[u8]) -> Obj {
    assert_eq!(u16_at(b, 0), 0x8664);
    let nsec = u16_at(b, 2) as usize;
    let (symoff, nsyms) = (u32_at(b, 8) as usize, u32_at(b, 12) as usize);
    let strtab = &b[symoff + 18 * nsyms..];
    let long = |n: &[u8]| -> String {
        if u32_at(n, 0) == 0 {
            cstr(strtab, u32_at(n, 4) as usize)
        } else {
            fixed(n)
        }
    };
    let sec_name = |n: &[u8]| -> String {
        match n[0] {
            b'/' => cstr(strtab, fixed(&n[1..]).parse().unwrap()),
            _ => fixed(n),
        }
    };
    let sym = |i: usize| &b[symoff + 18 * i..symoff + 18 * (i + 1)];
    let mut sections = Vec::new();
    for s in 0..nsec {
        let h = &b[20 + 40 * s..60 + 40 * s];
        let (size, ptr) = (u32_at(h, 16) as usize, u32_at(h, 20) as usize);
        let (rptr, nrel) = (u32_at(h, 24) as usize, u16_at(h, 32) as usize);
        let relocs = (0..nrel)
            .map(|i| {
                let r = &b[rptr + 10 * i..rptr + 10 * (i + 1)];
                let s = sym(u32_at(r, 4) as usize);
                assert_eq!(s[16], 3, "a section symbol");
                let target = &b[20 + 40 * (u16_at(s, 12) as usize - 1)..];
                (
                    u64::from(u32_at(r, 0)),
                    u32::from(u16_at(r, 8)),
                    sec_name(&target[..8]),
                )
            })
            .collect();
        sections.push(Sec {
            name: sec_name(&h[..8]),
            data: b[ptr..ptr + size].to_vec(),
            relocs,
        });
    }
    let mut globals = Vec::new();
    let mut i = 0;
    while i < nsyms {
        let s = sym(i);
        if s[16] == 2 {
            let sect = u16_at(s, 12) as usize;
            globals.push((long(&s[..8]), sections[sect - 1].name.clone()));
        } else {
            assert_eq!(long(&s[..8]), sections[u16_at(s, 12) as usize - 1].name);
        }
        i += 1 + s[17] as usize;
    }
    Obj { sections, globals }
}

fn read(format: ObjFormat, b: &[u8]) -> Obj {
    match format {
        ObjFormat::Elf => read_elf(b),
        ObjFormat::MachO => read_macho(b),
        ObjFormat::Coff => read_coff(b),
    }
}

// ---------------------------------------------------------------------------------------------

#[test]
fn each_target_object_has_its_sections_and_no_text_relocation() {
    let host = HostInfo::current();
    for (arch, abi, format) in TARGETS {
        let (unit, text) = unit_for(&host, arch, abi);
        let o = write_object(&unit, "ep9_unit_obj_test").unwrap();
        let what = format!("{format:?} arch {arch}");
        // Kept for a look with other tools (llvm-readobj, a linker).
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ep9_unit_obj_targets");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{format:?}-{arch}.o")), &o).unwrap();
        let obj = read(format, &o);
        let [t, u, r, w] = section_names(format);
        assert_eq!(obj.sec(t).data, text, "{what}: the text, unchanged");
        assert_eq!(
            obj.sec(t).relocs,
            [],
            "{what}: zero text relocations (H35 S5)"
        );
        assert_eq!(
            obj.sec(u).data,
            unit,
            "{what}: the whole container, unchanged"
        );
        assert_eq!(obj.sec(u).relocs, [], "{what}");
        let rec = &obj.sec(r).data;
        assert_eq!(rec.len(), 48, "{what}");
        assert_eq!(&rec[..16], b"EP9ULNK\0\x01\0\0\0\0\0\0\0", "{what}");
        assert_eq!(u64_at(rec, 24), unit.len() as u64, "{what}");
        assert_eq!(u64_at(rec, 40), text.len() as u64, "{what}");
        let mut rr: Vec<(u64, String)> = obj
            .sec(r)
            .relocs
            .iter()
            .map(|x| (x.0, x.2.clone()))
            .collect();
        rr.sort();
        assert_eq!(
            rr,
            [(16, u.to_string()), (32, t.to_string())],
            "{what}: record"
        );
        let abs64 = match (format, arch) {
            (ObjFormat::Elf, target_arch::X86_64) => 1,
            (ObjFormat::Elf, _) => 257,
            (ObjFormat::MachO, _) => 0,
            (ObjFormat::Coff, _) => 1,
        };
        assert!(
            obj.sec(r).relocs.iter().all(|x| x.1 == abs64),
            "{what}: {:?}",
            obj.sec(r)
        );
        let unwind = &obj.sec(w);
        let kinds: Vec<(u32, &str)> = unwind.relocs.iter().map(|x| (x.1, x.2.as_str())).collect();
        let want: Vec<(u32, &str)> = match (format, arch) {
            (ObjFormat::Elf, target_arch::X86_64) => vec![(2, t)],
            (ObjFormat::Elf, _) => vec![(261, t)],
            (ObjFormat::MachO, target_arch::X86_64) => vec![],
            (ObjFormat::MachO, _) => vec![(1, w), (0, t)],
            (ObjFormat::Coff, _) => vec![(3, t); 3],
        };
        assert_eq!(kinds, want, "{what}: unwind relocations");
        assert_eq!(
            obj.globals,
            [(
                if format == ObjFormat::MachO {
                    "_ep9_unit_obj_test"
                } else {
                    "ep9_unit_obj_test"
                }
                .to_string(),
                r.to_string()
            )],
            "{what}: the record symbol"
        );
        if format == ObjFormat::Elf {
            assert!(obj.names().contains(&".note.GNU-stack"), "{what}");
        }
        if format == ObjFormat::MachO {
            let a64 = arch == target_arch::AARCH64;
            let mac = if a64 {
                11 << 16
            } else {
                (10 << 16) | (12 << 8)
            };
            assert_eq!(macho_build_version(&o), Some((1, mac)), "{what}: macOS");
            let ios = write_object_for(&unit, "s", AppleOs::Ios).unwrap();
            assert_eq!(
                macho_build_version(&ios),
                Some((2, 10 << 16)),
                "{what}: iOS"
            );
            let sim = write_object_for(&unit, "s", AppleOs::IosSimulator).unwrap();
            let sim_min = if a64 { 14 << 16 } else { 10 << 16 };
            assert_eq!(
                macho_build_version(&sim),
                Some((7, sim_min)),
                "{what}: simulator"
            );
        }
    }
}

#[test]
fn the_writer_refuses_what_has_no_object_form() {
    let host = HostInfo::current();
    let (unit, _) = unit_for(&host, target_arch::AARCH64, target_abi::LINUX);
    for bad in ["", "1abc", "a-b", "a.b", "a b"] {
        assert_eq!(write_object(&unit, bad), Err(ObjError::Symbol), "{bad:?}");
    }
    let mut broken = unit.clone();
    *broken.last_mut().unwrap() ^= 1;
    assert_eq!(write_object(&broken, "s"), Err(ObjError::NotAUnit));
    let (mut win_a64, _) = unit_for(&host, target_arch::X86_64, target_abi::WINDOWS);
    win_a64 = {
        let mut u = TestUnit::parse(&win_a64).unwrap();
        u.header.target_arch = target_arch::AARCH64;
        u.encode()
    };
    assert_eq!(
        write_object(&win_a64, "s"),
        Err(ObjError::Target {
            arch: target_arch::AARCH64,
            abi: target_abi::WINDOWS
        })
    );
    let no_text = TestUnit::parse(&unit)
        .unwrap()
        .without(SectionKind::Text)
        .encode();
    assert_eq!(write_object(&no_text, "s"), Err(ObjError::NoText));
    // An FDE past the text; a CIE with another pointer encoding; a RUNTIME_FUNCTION past the
    // text.
    let with_unwind = |u: &[u8], f: &dyn Fn(&mut Vec<u8>)| {
        let mut t = TestUnit::parse(u).unwrap();
        for (k, d) in &mut t.sections {
            if *k == SectionKind::Unwind {
                f(d);
            }
        }
        t.encode()
    };
    let past = with_unwind(&unit, &|d| d[40] += 1);
    assert!(matches!(write_object(&past, "s"), Err(ObjError::Unwind(_))));
    let pcrel = with_unwind(&unit, &|d| d[16] = 0x1b);
    assert!(matches!(
        write_object(&pcrel, "s"),
        Err(ObjError::Unwind(_))
    ));
    let (win, _) = unit_for(&host, target_arch::X86_64, target_abi::WINDOWS);
    let win_past = with_unwind(&win, &|d| d[4] = 25);
    assert!(matches!(
        write_object(&win_past, "s"),
        Err(ObjError::Unwind(_))
    ));
}

#[test]
fn check_linked_needs_the_same_text() {
    let host = HostInfo::current();
    let (unit, text) = unit_for(&host, host.target_arch, host.target_abi);
    assert_eq!(check(&unit, &host), Outcome::Load);
    assert_eq!(check_linked(&unit, &text, &host), Outcome::Load);
    let mut other = text.clone();
    other[0] ^= 1;
    assert_eq!(check_linked(&unit, &other, &host), Outcome::Bytecode);
    assert_eq!(check_linked(&unit, &text[1..], &host), Outcome::Bytecode);
    let mut broken = unit.clone();
    *broken.last_mut().unwrap() ^= 1;
    assert_eq!(check_linked(&broken, &text, &host), check(&broken, &host));
}

/// Runs `cmd` to its end or kills it at the deadline (a hang guard, not a time bound).
fn run(cmd: &mut Command, what: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() > deadline {
            child.kill().ok();
            panic!("{what}: no exit before the deadline");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{what}: {}\n{text}", out.status);
    text
}

const PROGRAM: &str = r#"
use std::sync::atomic::{AtomicU32, Ordering};
#[repr(C)]
struct LinkedUnit {
    magic: [u8; 8],
    version: u32,
    reserved: u32,
    unit: *const u8,
    unit_len: u64,
    text: *const u8,
    text_len: u64,
}
extern "C" {
    static ep9_unit_obj_test: LinkedUnit;
}
static CALLS: AtomicU32 = AtomicU32::new(0);
extern "C-unwind" fn count() {
    CALLS.fetch_add(1, Ordering::SeqCst);
}
extern "C-unwind" fn boom() {
    std::panic::panic_any(7u32);
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (unit, text) = (std::fs::read(&a[1]).unwrap(), std::fs::read(&a[2]).unwrap());
    let r = unsafe { &*std::ptr::addr_of!(ep9_unit_obj_test) };
    assert_eq!((&r.magic, r.version, r.reserved), (b"EP9ULNK\0", 1, 0));
    let u = unsafe { std::slice::from_raw_parts(r.unit, r.unit_len as usize) };
    let t = unsafe { std::slice::from_raw_parts(r.text, r.text_len as usize) };
    assert!(u == &unit[..], "the record's container");
    assert!(t == &text[..], "the record's text");
    let f: extern "C-unwind" fn(extern "C-unwind" fn()) = unsafe { std::mem::transmute(r.text) };
    f(count);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    std::panic::set_hook(Box::new(|_| {}));
    let e = std::panic::catch_unwind(|| f(boom)).unwrap_err();
    assert_eq!(e.downcast_ref::<u32>(), Some(&7));
    println!("linked unit: ran in place, unwound through");
}
"#;

#[test]
fn the_object_links_and_runs_on_the_host() {
    let host = HostInfo::current();
    let tier1 = matches!(
        (host.target_arch, host.target_abi),
        (
            target_arch::X86_64 | target_arch::AARCH64,
            target_abi::LINUX | target_abi::APPLE
        ) | (target_arch::X86_64, target_abi::WINDOWS)
    );
    if !tier1 {
        eprintln!("skip: no .o form for this host");
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ep9_unit_obj");
    std::fs::create_dir_all(&dir).unwrap();
    let (unit, text) = unit_for(&host, host.target_arch, host.target_abi);
    let obj = dir.join(if cfg!(windows) { "unit.obj" } else { "unit.o" });
    std::fs::write(&obj, write_object(&unit, "ep9_unit_obj_test").unwrap()).unwrap();
    std::fs::write(dir.join("unit.bin"), &unit).unwrap();
    std::fs::write(dir.join("text.bin"), &text).unwrap();
    std::fs::write(dir.join("main.rs"), PROGRAM).unwrap();
    let exe = dir.join(if cfg!(windows) {
        "linked.exe"
    } else {
        "linked"
    });
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    // The package directory: rustup takes the toolchain of rust-toolchain.toml.
    let linked = run(
        Command::new(rustc)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .arg("--edition=2021")
            .arg("-Cpanic=unwind")
            .arg(format!("-Clink-arg={}", obj.display()))
            .arg("-o")
            .arg(&exe)
            .arg(dir.join("main.rs")),
        "rustc",
    );
    assert!(!linked.contains("warning"), "the link warns:\n{linked}");
    let out = run(
        Command::new(&exe)
            .arg(dir.join("unit.bin"))
            .arg(dir.join("text.bin")),
        "the linked program",
    );
    assert!(out.contains("ran in place, unwound through"), "{out}");
}
