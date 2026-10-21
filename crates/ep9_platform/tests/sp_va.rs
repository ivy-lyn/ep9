//! SP-VA gate (M0 week 2, H7): the cage and the process code range are reserved below 2^47,
//! neither overlaps the tsrs range, a cage has no code range of its own by default, and pages
//! commit, decommit and fault as expected.

use ep9_platform::va::{
    self, CAGE_LIMIT, Cage, CageConfig, CageError, CodeRange, CodeRangeConfig, DEFAULT_CAGE_ALIGN,
    DEFAULT_CODE_RANGE_SIZE, DEFAULT_MIN_BASE, Reservation, TSRS_RESERVATION,
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
    let h = cage.heap().range();
    println!(
        "SP-VA {}-{}: page {} KiB, granularity {} KiB; cage {:#x}..{:#x} ({} GiB); \
         end {:#x} < 2^47 = {:#x}; slots tried {}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        va::page_size() >> 10,
        va::allocation_granularity() >> 10,
        h.start,
        h.end,
        h.len() >> 30,
        h.end,
        CAGE_LIMIT,
        cage.slots_tried(),
    );
    assert!(h.end <= CAGE_LIMIT, "cage ends above 2^47");
    assert!(
        cage.code_range().is_none(),
        "the default cage has no code range"
    );
    assert_eq!(h.len(), config.cage_size);
    assert!(h.start.is_multiple_of(DEFAULT_CAGE_ALIGN));
    assert!(!overlaps(&h, &TSRS_RESERVATION));
    cage.self_check().unwrap();
    assert!(cage.contains(h.start) && cage.contains(h.end - 1));
    assert!(!cage.contains(h.end) && !cage.contains(h.start - 1));

    let ps = va::page_size();
    let r = cage.heap();
    touch(r, 0);
    touch(r, r.size() - ps);
}

/// The process code range (H07-Q6): one per process, 128 MiB, below 2^47, outside the tsrs
/// range. This test builds `ep9_platform` without `jit`, so the range is a plain reservation.
#[test]
fn sp_va_process_code_range_below_2_47() {
    let k = CodeRange::process().expect("reserve the process code range");
    println!(
        "SP-VA {}-{}: process code range {:#x}..{:#x} ({} MiB); slots tried {}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        k.base(),
        k.end(),
        k.size() >> 20,
        k.slots_tried(),
    );
    assert!(std::ptr::eq(k, CodeRange::process().unwrap()), "one range");
    assert_eq!(k.size(), DEFAULT_CODE_RANGE_SIZE);
    assert!(k.end() <= CAGE_LIMIT, "code range ends above 2^47");
    assert!(k.base().is_multiple_of(DEFAULT_CAGE_ALIGN));
    assert!(!overlaps(&k.range(), &TSRS_RESERVATION));
    k.self_check().unwrap();
    // A chunk is in the range, and the range is reserved: a fixed reserve there fails.
    let chunk = k.alloc(1).unwrap();
    assert!(k.contains(chunk.base()) && chunk.addr_range().end <= k.end());
    let g = va::allocation_granularity();
    assert!(Reservation::reserve_at(k.base(), g).unwrap().is_none());
}

/// The code-range search never tries a slot that overlaps the tsrs range either.
#[test]
fn sp_va_code_range_avoids_tsrs_range() {
    let first = DEFAULT_MIN_BASE / DEFAULT_CAGE_ALIGN;
    let tsrs_slot = TSRS_RESERVATION.start / DEFAULT_CAGE_ALIGN;
    let k = CodeRange::reserve(&CodeRangeConfig {
        seed: Some((tsrs_slot - first) as u64),
        ..CodeRangeConfig::default()
    })
    .unwrap();
    println!("seed at the tsrs slot: code range at {:#x}", k.base());
    assert!(!overlaps(&k.range(), &TSRS_RESERVATION));
    assert!(k.base() >= TSRS_RESERVATION.end);
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
    let span = cage.heap().range();
    println!(
        "seed at the tsrs slot {:#x}: cage at {:#x}",
        tsrs_slot * align,
        span.start
    );
    assert!(!overlaps(&span, &TSRS_RESERVATION));
    // The first slot after the tsrs range is the first one tried. Something else in the process
    // (other tests reserve cages at random slots) can hold it; then the search moves up from it.
    assert!(span.start >= TSRS_RESERVATION.end);
    if span.start == TSRS_RESERVATION.end {
        assert_eq!(cage.slots_tried(), 1);
    } else {
        assert!(cage.slots_tried() > 1);
    }
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

/// A slot that the host refuses with EINVAL is skipped, not an error (CI run 39505241524, perf
/// CI v0, exit 70). valgrind refuses a hinted mmap that overlaps its own mappings with EINVAL
/// when it has no other free range of that size below 128 GiB, so under valgrind the slots at
/// 4..128 GiB fail. A seccomp filter on a test thread makes the kernel do the same here: every
/// hinted `mmap` at a 4 GiB multiple in [4 GiB, 128 GiB) fails with EINVAL. Seed 0 starts the
/// search at 4 GiB, so the cage must land at 128 GiB or above.
#[cfg(target_os = "linux")]
#[test]
fn sp_va_skips_refused_slot() {
    const FIRST_OK: u32 = 32; // 128 GiB >> 32
    std::thread::spawn(|| {
        refuse_hinted_mmaps(1, FIRST_OK);
        // The filter works: a plain hinted mmap at 4 GiB fails with EINVAL.
        // SAFETY: a hinted PROT_NONE mapping changes no existing mapping; it fails here.
        let p = unsafe {
            libc::mmap(
                (4usize << 30) as *mut libc::c_void,
                4 << 30,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        assert_eq!(
            p,
            libc::MAP_FAILED,
            "the seccomp filter must refuse the hint"
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EINVAL)
        );
        let config = CageConfig {
            seed: Some(0),
            ..CageConfig::default()
        };
        let cage = Cage::reserve(&config).expect("refused slots must be skipped");
        println!(
            "cage at {:#x} after {} slots",
            cage.heap().base(),
            cage.slots_tried()
        );
        assert!(cage.heap().base() >= (FIRST_OK as usize) << 32);
        assert!(cage.slots_tried() > (FIRST_OK - 1) as usize);
        let refused = Reservation::reserve_at(4 << 30, 4 << 30).expect("EINVAL is not an error");
        assert!(refused.is_none(), "a refused range is not free");
    })
    .join()
    .unwrap();
}

/// Installs a seccomp filter on the calling thread (only): `mmap` with an address hint
/// `k << 32` for `k` in `[lo, hi)` fails with EINVAL. Other calls run as usual.
#[cfg(target_os = "linux")]
fn refuse_hinted_mmaps(lo: u32, hi: u32) {
    use libc::{
        BPF_ABS, BPF_JEQ, BPF_JGE, BPF_JMP, BPF_K, BPF_LD, BPF_RET, BPF_W, SECCOMP_RET_ALLOW,
        SECCOMP_RET_ERRNO, sock_filter,
    };
    let stmt = |code: u32, k: u32| sock_filter {
        code: code as u16,
        jt: 0,
        jf: 0,
        k,
    };
    let jump = |code: u32, k: u32, jt: u8, jf: u8| sock_filter {
        code: code as u16,
        jt,
        jf,
        k,
    };
    // struct seccomp_data: nr at 0, args[0] at 16 (low half) and 20 (high half; little endian).
    let mut prog = [
        stmt(BPF_LD | BPF_W | BPF_ABS, 0),
        jump(BPF_JMP | BPF_JEQ | BPF_K, libc::SYS_mmap as u32, 0, 6),
        stmt(BPF_LD | BPF_W | BPF_ABS, 16),
        jump(BPF_JMP | BPF_JEQ | BPF_K, 0, 0, 4),
        stmt(BPF_LD | BPF_W | BPF_ABS, 20),
        jump(BPF_JMP | BPF_JGE | BPF_K, lo, 0, 2),
        jump(BPF_JMP | BPF_JGE | BPF_K, hi, 1, 0),
        stmt(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | libc::EINVAL as u32),
        stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
    ];
    let fprog = libc::sock_fprog {
        len: prog.len() as u16,
        filter: prog.as_mut_ptr(),
    };
    let one: libc::c_ulong = 1;
    let zero: libc::c_ulong = 0;
    // SAFETY: PR_SET_NO_NEW_PRIVS takes integer arguments only; it is needed for an
    // unprivileged filter and only limits privilege gains (setuid exec) of this thread.
    let rc = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, one, zero, zero, zero) };
    assert_eq!(
        rc,
        0,
        "PR_SET_NO_NEW_PRIVS: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: `fprog` points to `prog`, a valid BPF program that lives across the call (the
    // kernel copies it). Without SECCOMP_FILTER_FLAG_TSYNC it applies to this thread only.
    let rc = unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::c_ulong::from(libc::SECCOMP_MODE_FILTER),
            &fprog as *const libc::sock_fprog,
        )
    };
    assert_eq!(rc, 0, "PR_SET_SECCOMP: {}", std::io::Error::last_os_error());
}

/// Drop releases the cage: the same seed gets the same slot again.
#[test]
fn sp_va_drop_releases() {
    let config = CageConfig {
        seed: Some(2000),
        avoid: vec![],
        ..CageConfig::default()
    };
    let first = Cage::reserve(&config).unwrap();
    let base = first.heap().base();
    drop(first);
    let second = Cage::reserve(&config).unwrap();
    assert_eq!(second.heap().base(), base);
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
    assert!(cage.heap().end() <= 1 << 39);
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
    let config = CodeRangeConfig {
        size: va::DIRECT_BRANCH_REACH + g,
        ..Default::default()
    };
    assert!(matches!(
        CodeRange::reserve(&config),
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
