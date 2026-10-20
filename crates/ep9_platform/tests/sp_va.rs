//! SP-VA gate (M0 week 2, H7): the cage and the code range are reserved below 2^47, the code
//! range starts at the cage end, neither overlaps the tsrs range, and pages commit, decommit
//! and fault as expected.

use ep9_platform::va::{
    self, CAGE_LIMIT, Cage, CageConfig, CageError, DEFAULT_CAGE_ALIGN, DEFAULT_MIN_BASE,
    Reservation, TSRS_RESERVATION,
};

const PATTERN: u64 = 0xE9E9_0000_0000_0001;

/// Commit one page, check it reads zero, write and read it, decommit, commit again and check it
/// reads zero (the old page was dropped), then decommit.
fn touch(r: &Reservation, offset: usize) {
    let ps = va::page_size();
    let p = (r.base() + offset) as *mut u64;
    r.commit(offset, ps).unwrap();
    // SAFETY: the page at `p` was committed read-write just now and nothing else uses it.
    unsafe {
        assert_eq!(p.read_volatile(), 0, "a fresh page must read zero");
        p.write_volatile(PATTERN);
        assert_eq!(p.read_volatile(), PATTERN);
        r.decommit(offset, ps).unwrap();
    }
    r.commit(offset, ps).unwrap();
    // SAFETY: committed again just now; no reference into the page exists.
    unsafe {
        assert_eq!(p.read_volatile(), 0, "decommit must drop the old page");
        r.decommit(offset, ps).unwrap();
    }
}

fn overlaps(a: &std::ops::Range<usize>, b: &std::ops::Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

#[test]
fn sp_va_default_cage_below_2_47() {
    let config = CageConfig::default();
    let cage = Cage::reserve(&config).expect("reserve the default cage");
    let (h, k) = (cage.heap().range(), cage.code().range());
    println!(
        "SP-VA {}-{}: page {} KiB, granularity {} KiB; cage {:#x}..{:#x} ({} GiB); \
         code range {:#x}..{:#x} ({} MiB); end {:#x} < 2^47 = {:#x}; slots tried {}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        va::page_size() >> 10,
        va::allocation_granularity() >> 10,
        h.start,
        h.end,
        h.len() >> 30,
        k.start,
        k.end,
        k.len() >> 20,
        k.end,
        CAGE_LIMIT,
        cage.slots_tried(),
    );
    assert!(k.end <= CAGE_LIMIT, "code range ends above 2^47");
    assert_eq!(k.start, h.end, "code range must start at the cage end");
    assert_eq!(h.len(), config.cage_size);
    assert_eq!(k.len(), config.code_range_size);
    assert!(h.start.is_multiple_of(DEFAULT_CAGE_ALIGN));
    assert!(!overlaps(&(h.start..k.end), &TSRS_RESERVATION));
    cage.self_check().unwrap();
    assert!(cage.contains(h.start) && cage.contains(h.end - 1));
    assert!(!cage.contains(h.end) && !cage.contains(h.start - 1));

    let ps = va::page_size();
    for r in [cage.heap(), cage.code()] {
        touch(r, 0);
        touch(r, r.size() - ps);
    }
}

/// The search never tries a slot that overlaps the tsrs range: the seed points at it.
#[test]
fn sp_va_avoids_tsrs_range() {
    let align = DEFAULT_CAGE_ALIGN;
    let first = DEFAULT_MIN_BASE / align;
    let tsrs_slot = TSRS_RESERVATION.start / align;
    let config = CageConfig {
        seed: Some((tsrs_slot - first) as u64),
        ..CageConfig::default()
    };
    let cage = Cage::reserve(&config).unwrap();
    let span = cage.heap().base()..cage.code().end();
    println!(
        "seed at the tsrs slot {:#x}: cage at {:#x}",
        tsrs_slot * align,
        span.start
    );
    assert!(!overlaps(&span, &TSRS_RESERVATION));
    // The first slot after the tsrs range is the first one tried.
    assert_eq!(span.start, TSRS_RESERVATION.end);
    assert_eq!(cage.slots_tried(), 1);
}

/// A slot that is in use is skipped (unix: the kernel moves the mapping and we give it back;
/// Windows: VirtualAlloc fails with ERROR_INVALID_ADDRESS).
#[test]
fn sp_va_skips_used_slot() {
    let align = DEFAULT_CAGE_ALIGN;
    let first = DEFAULT_MIN_BASE / align;
    let seed = 1000u64;
    let slot_base = (first + seed as usize) * align;
    let blocker = Reservation::reserve_at(slot_base, align)
        .unwrap()
        .expect("the test slot must be free");
    let config = CageConfig {
        seed: Some(seed),
        avoid: vec![],
        ..CageConfig::default()
    };
    let cage = Cage::reserve(&config).unwrap();
    println!(
        "blocker {:#x}..{:#x}; cage at {:#x} after {} slots",
        blocker.base(),
        blocker.end(),
        cage.heap().base(),
        cage.slots_tried()
    );
    assert_eq!(cage.heap().base(), slot_base + align);
    assert_eq!(cage.slots_tried(), 2);
    // The search must not have released the blocker: its range is still in use.
    assert!(
        Reservation::reserve_at(blocker.base(), blocker.size())
            .unwrap()
            .is_none()
    );
}

/// Drop releases both parts: the same seed gets the same slot again.
#[test]
fn sp_va_drop_releases() {
    let config = CageConfig {
        seed: Some(2000),
        avoid: vec![],
        ..CageConfig::default()
    };
    let first = Cage::reserve(&config).unwrap();
    let (base, code) = (first.heap().base(), first.code().base());
    drop(first);
    let second = Cage::reserve(&config).unwrap();
    assert_eq!((second.heap().base(), second.code().base()), (base, code));
}

/// A lower limit, as on a 39-bit VA kernel, still works. The cage is 4 GiB here: macOS arm64
/// keeps most of the space below 448 GiB for the shared region and a 384 GiB system
/// reservation, so a 64 GiB cage does not fit below 2^39 there.
#[test]
fn sp_va_low_limit() {
    let config = CageConfig {
        cage_size: 4 << 30,
        limit: 1 << 39,
        ..CageConfig::default()
    };
    let cage = Cage::reserve(&config).unwrap();
    println!("limit 2^39: cage at {:#x}", cage.heap().base());
    assert!(cage.code().end() <= 1 << 39);
}

#[test]
fn sp_va_rejects_bad_input() {
    let g = va::allocation_granularity();
    let ps = va::page_size();
    assert!(Reservation::reserve(0).is_err());
    assert!(Reservation::reserve_at(DEFAULT_MIN_BASE + g / 2, g).is_err());
    let r = Reservation::reserve(4 * g).unwrap();
    assert!(r.commit(0, 0).is_err());
    assert!(r.commit(1, ps).is_err());
    assert!(r.commit(r.size(), ps).is_err());
    assert!(r.commit(r.size() - ps, 2 * ps).is_err());
    let config = CageConfig {
        code_range_size: va::DIRECT_BRANCH_REACH + g,
        ..Default::default()
    };
    assert!(matches!(
        Cage::reserve(&config),
        Err(CageError::InvalidConfig(_))
    ));
}

const CHILD_ENV: &str = "EP9_SP_VA_CHILD";

/// Run in a child process by `sp_va_uncommitted_page_faults`: reads a page that is reserved
/// but not committed. This must kill the process.
#[test]
fn sp_va_child_reads_uncommitted_page() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let cage = Cage::reserve(&CageConfig::default()).unwrap();
    let ps = va::page_size();
    cage.heap().commit(0, ps).unwrap();
    let committed = cage.heap().base() as *const u64;
    let reserved = (cage.heap().base() + ps) as *const u64;
    // SAFETY: page 0 is committed.
    assert_eq!(unsafe { committed.read_volatile() }, 0);
    eprintln!("child: before fault");
    // SAFETY: not safe on purpose. The page is reserved without access, so the read faults
    // and the child process dies. This is what the parent test checks.
    let v = unsafe { reserved.read_volatile() };
    eprintln!("child: read {v:#x} from an uncommitted page");
}

#[test]
fn sp_va_uncommitted_page_faults() {
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(exe)
        .args([
            "sp_va_child_reads_uncommitted_page",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    println!("child status: {:?}", out.status);
    assert!(
        stderr.contains("child: before fault"),
        "child did not reach the read: {stderr}"
    );
    assert!(
        !stderr.contains("from an uncommitted page"),
        "the read did not fault: {stderr}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        let sig = out.status.signal();
        assert!(
            sig == Some(libc::SIGSEGV) || sig == Some(libc::SIGBUS),
            "child must die from SIGSEGV or SIGBUS, got {:?}",
            out.status
        );
    }
    #[cfg(windows)]
    assert_eq!(
        out.status.code(),
        Some(0xC000_0005_u32 as i32),
        "expected an access violation"
    );
}
