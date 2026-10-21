//! Encoder tests (every host). The registration and walk tests that need generated code are in
//! `jit::tests`.

use super::cfi::{self, CfiProgram, Cie, EhFrameBuilder, EhFrameSection, reg};
use super::win64::{RuntimeFunction, UnwindInfoBuilder, reg as wreg};

#[test]
fn leb128() {
    assert_eq!(cfi::uleb_for_test(2), [2]);
    assert_eq!(cfi::uleb_for_test(127), [0x7f]);
    assert_eq!(cfi::uleb_for_test(128), [0x80, 0x01]);
    assert_eq!(cfi::uleb_for_test(624_485), [0xe5, 0x8e, 0x26]);
    assert_eq!(cfi::sleb_for_test(2), [2]);
    assert_eq!(cfi::sleb_for_test(-1), [0x7f]);
    assert_eq!(cfi::sleb_for_test(-8), [0x78]);
    assert_eq!(cfi::sleb_for_test(63), [0x3f]);
    assert_eq!(cfi::sleb_for_test(64), [0xc0, 0x00]);
    assert_eq!(cfi::sleb_for_test(-64), [0x40]);
    assert_eq!(cfi::sleb_for_test(-65), [0xbf, 0x7f]);
    assert_eq!(cfi::sleb_for_test(-123_456), [0xc0, 0xbb, 0x78]);
}

#[test]
fn cfi_instructions() {
    let mut p = CfiProgram::new();
    p.advance_loc(1)
        .def_cfa_offset(32)
        .offset(reg::aarch64::FP, 4)
        .offset(reg::aarch64::LR, 3)
        .advance_loc(1)
        .def_cfa_register(reg::aarch64::FP)
        .restore(reg::aarch64::X19)
        .def_cfa(reg::aarch64::SP, 0)
        .advance_loc(0x40)
        .advance_loc(0x100)
        .advance_loc(0x1_0000)
        .offset(70, 1)
        .restore(70)
        .remember_state()
        .restore_state();
    let want: &[u8] = &[
        0x41, 0x0e, 32, 0x9d, 4, 0x9e, 3, // loc+1; cfa off 32; x29@-32; x30@-24
        0x41, 0x0d, 29,   // loc+1; cfa reg x29
        0xd3, // restore x19
        0x0c, 31, 0, // cfa = sp + 0
        0x02, 0x40, // advance_loc1
        0x03, 0x00, 0x01, // advance_loc2
        0x04, 0x00, 0x00, 0x01, 0x00, // advance_loc4
        0x05, 70, 1, // offset_extended
        0x06, 70, // restore_extended
        0x0a, 0x0b,
    ];
    assert_eq!(p.bytes(), want);
}

#[test]
fn eh_frame_layout_aarch64() {
    let mut p = CfiProgram::new();
    p.advance_loc(1).def_cfa_offset(16);
    let mut b = EhFrameBuilder::new(&Cie::aarch64());
    b.add_fde(0x1122_3344_5566_7788, 0x28, &p);
    let s = b.finish();
    let bytes = s.bytes();
    #[rustfmt::skip]
    let cie: &[u8] = &[
        20, 0, 0, 0,  0, 0, 0, 0,  1, b'z', b'R', 0,  4, 0x78, 30, 1, 0,  0x0c, 31, 0,  0, 0, 0, 0,
    ];
    assert_eq!(&bytes[..24], cie);
    // FDE at 24: length, CIE pointer = 28 (back to offset 0), pc_begin, pc_range, aug 0.
    assert_eq!(&bytes[24..28], &28u32.to_le_bytes()); // 4+4+8+8+1+3 = 28 -> 32; len = 28
    assert_eq!(&bytes[28..32], &28u32.to_le_bytes());
    assert_eq!(&bytes[32..40], &0x1122_3344_5566_7788u64.to_le_bytes());
    assert_eq!(&bytes[40..48], &0x28u64.to_le_bytes());
    assert_eq!(&bytes[48..52], &[0, 0x41, 0x0e, 16]);
    assert_eq!(&bytes[52..56], &[0, 0, 0, 0]); // nop padding
    assert_eq!(&bytes[56..], &[0, 0, 0, 0]); // terminator
    assert_eq!(s.records().unwrap(), [(0, true), (24, false)]);
    assert_eq!(
        s.bytes().as_ptr() as usize % 8,
        0,
        "section storage is 8-byte aligned"
    );
}

#[test]
fn eh_frame_layout_x86_64() {
    let mut b = EhFrameBuilder::new(&Cie::x86_64());
    b.add_fde(0x1000, 16, &CfiProgram::new());
    b.add_fde(0x2000, 16, &CfiProgram::new());
    let s = b.finish();
    let bytes = s.bytes();
    #[rustfmt::skip]
    let cie: &[u8] = &[
        20, 0, 0, 0,  0, 0, 0, 0,  1, b'z', b'R', 0,  1, 0x78, 16, 1, 0,  0x0c, 7, 8,  0x90, 1, 0, 0,
    ];
    assert_eq!(&bytes[..24], cie);
    // Each FDE: 4 + 4 + 8 + 8 + 1 = 25 -> 32 bytes. The second points 4 + 56 bytes back.
    assert_eq!(s.records().unwrap(), [(0, true), (24, false), (56, false)]);
    assert_eq!(&bytes[60..64], &60u32.to_le_bytes());
    assert_eq!(bytes.len(), 24 + 32 + 32 + 4);
}

#[test]
fn eh_frame_rejects_bad_sections() {
    // No terminator.
    assert!(EhFrameSection::from_bytes(&[8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]).is_err());
    // Record longer than the section.
    assert!(EhFrameSection::from_bytes(&[64, 0, 0, 0, 0, 0, 0, 0]).is_err());
    // FDE whose CIE pointer does not hit a CIE.
    let mut b = EhFrameBuilder::new(&Cie::x86_64());
    b.add_fde(0x1000, 16, &CfiProgram::new());
    let mut bytes = b.finish().bytes().to_vec();
    bytes[28] = 12;
    assert!(EhFrameSection::from_bytes(&bytes).is_err());
    // 64-bit length form.
    assert!(EhFrameSection::from_bytes(&[0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0]).is_err());
    // Empty section (terminator only) is fine.
    assert_eq!(
        EhFrameSection::from_bytes(&[0, 0, 0, 0])
            .unwrap()
            .records()
            .unwrap(),
        []
    );
}

#[test]
fn win64_unwind_info() {
    // push rbx (ends at 1); sub rsp, 32 (ends at 5). Same bytes as llvm-mc gives for
    // .seh_pushreg rbx / .seh_stackalloc 32 (checked 2026-10-21).
    let mut b = UnwindInfoBuilder::new();
    b.push_nonvol(1, wreg::RBX).alloc(5, 32);
    assert_eq!(b.encode(), [1, 5, 2, 0, 0x05, 0x32, 0x01, 0x30]);

    // push rbp; lea rbp, [rsp+32]; sub rsp, 0x200; mov [rsp+8], rsi: odd count gets padding.
    let mut b = UnwindInfoBuilder::new();
    b.push_nonvol(1, wreg::RBP)
        .set_fpreg(6, wreg::RBP, 32)
        .alloc(13, 0x200)
        .save_nonvol(18, wreg::RSI, 8);
    #[rustfmt::skip]
    let want = [
        1, 18, 6, 0x25,
        18, 0x64, 1, 0,       // SAVE_NONVOL rsi, slot = 8 / 8
        13, 0x01, 0x40, 0,    // ALLOC_LARGE info 0, slot = 0x200 / 8
        6, 0x03,              // SET_FPREG
        1, 0x50,              // PUSH_NONVOL rbp
    ];
    assert_eq!(b.encode(), want);

    // Large allocation with a 32-bit size.
    let mut b = UnwindInfoBuilder::new();
    b.alloc(7, 0x8_0008);
    assert_eq!(
        b.encode(),
        [1, 7, 3, 0, 7, 0x11, 0x08, 0x00, 0x08, 0x00, 0, 0]
    );
}

#[test]
fn win64_runtime_function_layout() {
    assert_eq!(std::mem::size_of::<RuntimeFunction>(), 12);
    assert_eq!(std::mem::align_of::<RuntimeFunction>(), 4);
}

#[test]
fn native_walk_sees_this_function() {
    #[inline(never)]
    fn inner() -> Vec<usize> {
        std::hint::black_box(super::native_return_addresses(64))
    }
    let ips = inner();
    assert!(
        ips.len() >= 3,
        "the walk found only {} frames: {ips:x?}",
        ips.len()
    );
    // A return address into this test function lies within a few KiB after its start.
    let me = native_walk_sees_this_function as fn() as usize;
    assert!(
        ips.iter().any(|&ip| ip > me && ip - me < 16 << 10),
        "no frame returns into the test function {me:#x}: {ips:x?}"
    );
}

#[test]
fn native_return_addresses_honors_max() {
    assert_eq!(super::native_return_addresses(0), []);
    assert_eq!(super::native_return_addresses(1).len(), 1);
    assert_eq!(super::native_return_addresses(2).len(), 2);
}

// ---------------------------------------------------------------------------------------------
// Registration mode self-test (unix).

/// The process's mode. Prints it for CI logs. `EP9_EXPECT_UNWIND_MODE=whole-section|per-fde`
/// makes the test fail on any other mode (the x86 libgcc and zig builds set it).
#[cfg(unix)]
#[test]
fn selected_registration_mode() {
    let mode = super::registration_mode().expect("a registration mode passes the self-test");
    eprintln!(
        "unwind registration mode: {mode} (dlsym guess: {})",
        super::dlsym_guess()
    );
    if let Ok(want) = std::env::var("EP9_EXPECT_UNWIND_MODE") {
        assert_eq!(mode.name(), want, "EP9_EXPECT_UNWIND_MODE");
    }
    // Apple libunwind (libSystem) takes one FDE per call.
    #[cfg(target_vendor = "apple")]
    assert_eq!(mode, super::RegistrationMode::PerFde);
    // The choice is made once.
    assert_eq!(super::registration_mode().unwrap(), mode);
}

/// The built-in probe passes again with the chosen mode, and a registration uses that mode.
#[cfg(unix)]
#[test]
fn built_in_probe_and_register_use_the_mode() {
    let mode = super::registration_mode().unwrap();
    // SAFETY: `with_probe` maps the probe while the closure runs; the section describes it.
    let again = super::mode::with_probe(super::mode::probe_section, |p| unsafe {
        super::mode::select(p)
    })
    .unwrap();
    assert_eq!(again, Ok(mode));
    let r = super::mode::with_probe(super::mode::probe_section, |p| {
        // SAFETY: as above; the registration drops inside the closure, before the page.
        let reg = unsafe { super::EhFrameRegistration::register(p.section.clone()) }.unwrap();
        reg.mode()
    })
    .unwrap();
    assert_eq!(r, mode);
}

/// `self_test` on a caller's probe: the mode is chosen already, so it checks the probe in it.
#[cfg(unix)]
#[test]
fn self_test_checks_a_probe() {
    let mode = super::registration_mode().unwrap();
    let r = super::mode::with_probe(super::mode::probe_section, |p| {
        let probe = super::SelfTestProbe {
            entry: p.entry,
            code: p.code.clone(),
            section: p.section.clone(),
        };
        // SAFETY: the probe is mapped while the closure runs; the section describes it.
        unsafe { super::self_test(probe) }
    })
    .unwrap();
    assert_eq!(r.unwrap(), mode);
}

/// A section without the probe's FDE: both modes miss, and the error names both.
#[cfg(unix)]
#[test]
fn missing_fde_fails_both_modes() {
    use super::cfi::{CfiProgram, Cie, EhFrameBuilder};
    let no_probe_fde = |base: usize, _len: usize| {
        let mut b = EhFrameBuilder::new(&Cie::host());
        b.add_fde(base + super::mode::OTHER_OFF, 1, &CfiProgram::new());
        b.finish()
    };
    // SAFETY: the probe is mapped while the closure runs; the one FDE describes the `other`
    // function (a bare return: the CIE rules hold at its only instruction).
    let r = super::mode::with_probe(no_probe_fde, |p| unsafe { super::mode::select(p) }).unwrap();
    let e = r.expect_err("no mode can walk through a frame without an FDE");
    // libgcc reports the probe frame and stops there; Apple libunwind stops before it.
    let miss = |mode: &str| {
        e.contains(&format!("{mode}: the walk stops at the probe frame"))
            || e.contains(&format!("{mode}: no frame returns into the probe"))
    };
    assert!(
        miss("whole-section") && miss("per-fde") && e.contains("Native code cannot run"),
        "{e}"
    );
    // A failed check never sets the process mode (a test above may have set it already).
    assert!(super::registration_mode().is_ok());
}

/// The walk check on made-up walks.
#[cfg(unix)]
#[test]
fn walk_check_cases() {
    use super::mode::{MAX_FRAMES, check};
    let code = 0x1000..0x1028;
    let base = [0x50, 0x60, 0x7000, 0x8000, 0x9000];
    // callback frames, the probe return address, the return into `run`, then the caller's.
    let good = [0x10, 0x20, 0x1018, 0x7100, 0x8000, 0x9000];
    assert_eq!(check(&code, &base, Some(&good)), Ok(()));
    // A return address at the probe's end still counts (a call as the last instruction).
    let at_end = [0x10, 0x1028, 0x7100, 0x8000, 0x9000];
    assert_eq!(check(&code, &base, Some(&at_end)), Ok(()));
    let err = |w: &[usize]| check(&code, &base, Some(w)).unwrap_err();
    assert!(
        check(&code, &base, None)
            .unwrap_err()
            .contains("did not call")
    );
    assert!(err(&[0x10, 0x20, 0x7100]).contains("no frame returns into the probe"));
    // The probe start is not a return address into it.
    assert!(err(&[0x10, 0x1000, 0x7100, 0x8000]).contains("no frame returns"));
    assert!(err(&[0x10, 0x1018]).contains("stops at the probe frame"));
    assert!(err(&[0x10, 0x1018, 0x7100]).contains("stops at the probe frame"));
    assert!(err(&[0x10, 0x1018, 0x7100, 0x8888, 0x9000]).contains("does not reach"));
    // The tail must be a proper suffix: the baseline also has the frame into `run`.
    assert!(
        check(&code, &[0x8000, 0x9000], Some(&good))
            .unwrap_err()
            .contains("does not reach")
    );
    let long = vec![0x1018; MAX_FRAMES];
    assert!(err(&long).contains("did not end"));
}

/// Each mode on this host, printed for CI logs. The chosen mode passes, and every mode tried
/// before it misses.
#[cfg(unix)]
#[test]
fn each_mode_on_this_host() {
    use super::RegistrationMode;
    let chosen = super::registration_mode().unwrap();
    let results = super::mode::with_probe(super::mode::probe_section, |p| {
        RegistrationMode::ORDER.map(|m| {
            // SAFETY: the probe is mapped while the closure runs; the section describes it.
            (m, unsafe { super::mode::try_mode(p, m) })
        })
    })
    .unwrap();
    for (m, r) in &results {
        eprintln!(
            "unwind mode {m}: {}",
            r.as_ref().map_or_else(|e| e.as_str(), |()| "pass")
        );
    }
    let at = RegistrationMode::ORDER
        .iter()
        .position(|&m| m == chosen)
        .unwrap();
    assert!(results[at].1.is_ok(), "{results:?}");
    assert!(results[..at].iter().all(|(_, r)| r.is_err()), "{results:?}");
}
