//! The unit loader (M8-P4 A2-GATE, route UNIT-LOADER-FUZZ; plan v13 §7.1): unit-container
//! bytes through the H35 S4 checks (`ep9_unit::check`, steps 1-4) and, on `Load`, through the
//! embedder steps and the `ep9_vm` unit loader (`Cx::run_script_with_unit_code`), in a new
//! jitless isolate per input (`rt.units` grows on each run).
//!
//! ```text
//! input --pad (below)--> container --ENGINE_BUILD_ID, content hash--> check(bytes, this host)
//!   Load: bytecode section --decode_unit, verify--> VerifiedUnit     (refused: no run)
//!         text + rodata --functions::decode, the fuse-id lists--> the function table
//!                                                    (refused: no table, the bytecode runs)
//!         run_script_with_unit_code; a watchdog thread terminates the run after RUN_BOUND
//!         the counts: "installed" 0 and "deferred" = "picked" (the safe method never installs)
//! crash = a panic or an abort anywhere (the H23 out-of-memory panics are not, below), an ASan
//!         report, a count that breaks the rule above, or a run past libFuzzer -timeout
//! ```
//!
//! Input form: a container with the zero bytes between the section table and its first section
//! cut out. The text section is 64 KiB aligned (container v3), and the fuzz runs use -max_len
//! 4096, so the target puts the zero bytes back: the first section then starts at the offset
//! that its table entry names (at most `PAD_MAX` bytes). Then it writes `ENGINE_BUILD_ID` into
//! the header and the content hash over the result (SHA-256 with the hash field and the
//! signature section zero), as the writer does: the build id hashes the commit
//! (`ep9_unit/build.rs`), so a committed seed would stop at step 3 on every later commit, and a
//! mutation would stop at step 2. Every other header field stays as the input has it, so the
//! target pair, the versions and the CPU bits are fuzzed: `fuzz/seeds/unit_loader` holds each
//! seed for aarch64 macOS, x86-64 Linux and aarch64 Linux. A seed whose versions are older than
//! this build stops at step 3 or 4 (rewrite the seeds when a contract version changes).
//!
//! The safe method maps no input byte as code and installs nothing (`abi/unit_code.rs` module
//! doc, route MC-w34-2), so the text bytes never run. The fuzzed bytecode does run (it passed
//! the verifier); the isolate has no builtin table, and `print` is a host function that does
//! nothing. The heap limit is `MAX_HEAP`: an allocation past it is fatal by design (H23 "Out of
//! memory": the heap limit, a feedback vector or a shape, `out_of_memory`), so the target ends
//! that run and drops its isolate.
//! The target never enables the `aot` feature of ep9_unit (dep-guards check 10).
#![no_main]

use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use ep9_bytecode::sha256::sha256;
use ep9_unit::functions;
use ep9_unit::{
    ENGINE_BUILD_ID, HostInfo, MAX_SECTIONS, Outcome, SECTION_ENTRY_SIZE, SECTION_TABLE_OFFSET,
    SectionKind, check, entry_offsets, offsets, unit_section,
};
use ep9_vm::{Args, Cx, ExecOptions, Isolate, IsolateConfig, JsResult, Local};
use libfuzzer_sys::fuzz_target;

/// The most zero bytes the target puts back before the first section: the 64 KiB text
/// alignment and some room.
const PAD_MAX: u64 = 1 << 17;
/// The time one run may take; the watchdog then terminates it (a fuzzed bytecode can loop).
const RUN_BOUND: Duration = Duration::from_millis(250);
/// The heap limit of each isolate (the libFuzzer -rss_limit_mb of run.sh is 4096).
const MAX_HEAP: usize = 256 << 20;
/// The panic text of the H23 heap limit (`ep9_vm` runtime.rs, `alloc_failed`).
const HEAP_LIMIT: &str = "ep9: heap limit reached";

/// True for an H23 out-of-memory panic under `MAX_HEAP`: the heap limit (`runtime.rs`
/// `alloc_failed`), a feedback vector (`feedback.rs`) or a shape (`shape.rs`).
fn out_of_memory(p: &(dyn std::any::Any + Send)) -> bool {
    let s = p
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| p.downcast_ref::<String>().map(String::as_str));
    s.is_some_and(|s| s == HEAP_LIMIT || s.ends_with(" allocation failed: OutOfMemory"))
}

fuzz_target!(|data: &[u8]| {
    let mut bytes = padded(data);
    if bytes.len() < SECTION_TABLE_OFFSET {
        // Too short for a header: check gives a LOAD ERROR (step 1).
        assert_eq!(check(&bytes, &HostInfo::current()), Outcome::LoadError);
        return;
    }
    let id = offsets::ENGINE_BUILD_ID;
    bytes[id..id + 16].copy_from_slice(&ENGINE_BUILD_ID);
    let hash = content_hash(&bytes);
    bytes[offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32].copy_from_slice(&hash);
    let host = HostInfo::current();
    if check(&bytes, &host) != Outcome::Load {
        return;
    }
    let section = |k| unit_section(&bytes, k);
    let Some(code) = section(SectionKind::Bytecode) else {
        panic!("a unit that loads has a bytecode section");
    };
    let Ok(unit) = ep9_bytecode::decode_unit(code) else {
        return;
    };
    let Ok(unit) = ep9_bytecode::verify(unit) else {
        return;
    };
    let text = section(SectionKind::Text).unwrap_or(&[]);
    let rodata = section(SectionKind::Rodata).unwrap_or(&[]);
    let entries = match functions::decode(rodata, text.len()) {
        Ok((entries, _)) => entries,
        Err(_) => Vec::new(),
    };
    // The fuse ids of each variant: `fuse_count` u32 values at `fuse_refs` in rodata (decode
    // checked the range).
    let fuses: Vec<Vec<Vec<u32>>> = entries
        .iter()
        .map(|e| {
            e.variants
                .iter()
                .map(|v| {
                    let at = v.fuse_refs as usize;
                    (0..usize::from(v.fuse_count))
                        .map(|k| {
                            let b = &rodata[at + 4 * k..at + 4 * k + 4];
                            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    type Var<'a> = (u8, u8, u32, u32, u64, &'a [u32]);
    let vars: Vec<Vec<Var<'_>>> = entries
        .iter()
        .zip(&fuses)
        .map(|(e, f)| {
            e.variants
                .iter()
                .zip(f)
                .map(|(v, ids)| {
                    let k = v.kind.code();
                    (k, v.lowering, v.entry, v.text_len, v.cpu_required, &ids[..])
                })
                .collect()
        })
        .collect();
    let fns: Vec<(u32, [u8; 16], &[Var<'_>])> = entries
        .iter()
        .zip(&vars)
        .map(|(e, v)| (e.function, e.digest, &v[..]))
        .collect();
    let mut rows = None;
    // The completion value and a throw (the terminate exception too) are both results.
    run_bounded(|cx| {
        let (counts, r) =
            cx.run_script_with_unit_code(unit, "unit.js", text, host.cpu_features, &fns);
        rows = Some(counts);
        r.map(|_| ())
    });
    if let Some(rows) = rows {
        let n = |name: &str| rows.iter().find(|(r, _)| *r == name).map_or(0, |r| r.1);
        assert_eq!(n("installed"), 0, "the safe method installed unit code");
        assert_eq!(n("deferred"), n("picked"), "a pick that is not deferred");
    }
});

/// `data` with the zero bytes put back before the first section (the input form, module doc).
fn padded(data: &[u8]) -> Vec<u8> {
    let le = |at: usize, n: usize| -> Option<u64> {
        let b = data.get(at..at + n)?;
        Some(b.iter().rev().fold(0u64, |a, &x| a << 8 | u64::from(x)))
    };
    let count = le(offsets::SECTION_COUNT, 4).unwrap_or(0);
    if count == 0 || count > u64::from(MAX_SECTIONS) {
        return data.to_vec();
    }
    let table_end = SECTION_TABLE_OFFSET + count as usize * SECTION_ENTRY_SIZE;
    let first = le(SECTION_TABLE_OFFSET + entry_offsets::OFFSET, 8).unwrap_or(0);
    let pad = first.saturating_sub(table_end as u64);
    if data.len() < table_end || pad > PAD_MAX {
        return data.to_vec();
    }
    let mut out = Vec::with_capacity(data.len() + pad as usize);
    out.extend_from_slice(&data[..table_end]);
    out.resize(table_end + pad as usize, 0);
    out.extend_from_slice(&data[table_end..]);
    out
}

/// SHA-256 of `bytes` with the content-hash field and the signature section (the table entry
/// of kind Signature, if its range is inside `bytes`) set to zero: the H35 S4 step-2 rule.
fn content_hash(bytes: &[u8]) -> [u8; 32] {
    let mut copy = bytes.to_vec();
    copy[offsets::CONTENT_HASH..offsets::CONTENT_HASH + 32].fill(0);
    let count = u32::from_le_bytes(
        bytes[offsets::SECTION_COUNT..offsets::SECTION_COUNT + 4]
            .try_into()
            .unwrap(),
    )
    .min(MAX_SECTIONS) as usize;
    for i in 0..count {
        let at = SECTION_TABLE_OFFSET + i * SECTION_ENTRY_SIZE;
        let Some(e) = bytes.get(at..at + SECTION_ENTRY_SIZE) else {
            break;
        };
        let u64_at = |o: usize| u64::from_le_bytes(e[o..o + 8].try_into().unwrap());
        let kind = u32::from_le_bytes(e[..4].try_into().unwrap());
        let (start, size) = (u64_at(entry_offsets::OFFSET), u64_at(entry_offsets::SIZE));
        let end = start.checked_add(size);
        if kind == SectionKind::Signature as u32
            && let Some(end) = end.filter(|&end| end <= bytes.len() as u64)
        {
            copy[start as usize..end as usize].fill(0);
        }
    }
    sha256(&copy)
}

/// Runs `f` on a new jitless isolate (heap limit `MAX_HEAP`, a no-op `print`) while a watchdog
/// thread terminates the run after `RUN_BOUND`. A throw and an H23 OOM panic end the run.
fn run_bounded(f: impl for<'s> FnOnce(&mut Cx<'s>) -> JsResult<()>) {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        // libfuzzer-sys's hook aborts on every panic; an H23 out-of-memory panic unwinds instead
        // (to `catch_unwind` below). Every other panic still goes to that hook.
        let fuzzer_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !out_of_memory(info.payload()) {
                fuzzer_hook(info);
            }
        }));
    });
    let mut config = IsolateConfig::default();
    config.heap.max_heap_bytes = MAX_HEAP;
    let mut iso =
        Isolate::new_with(config, ExecOptions::jitless()).expect("the jitless preset runs");
    iso.register_host_function("print", 1, print);
    let stop = iso.terminate_handle();
    let (done, wait) = mpsc::channel::<()>();
    std::thread::scope(|s| {
        s.spawn(move || {
            if wait.recv_timeout(RUN_BOUND) == Err(RecvTimeoutError::Timeout) {
                stop.terminate();
            }
        });
        let r = panic::catch_unwind(AssertUnwindSafe(|| iso.with_cx(f)));
        drop(done);
        match r {
            Ok(_) => {}
            // Then the isolate drops, not forgets (a leak per input); a probe drop was clean.
            Err(p) if out_of_memory(&*p) => {}
            Err(p) => panic::resume_unwind(p),
        }
    });
}

fn print<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _new_target: Local<'s>,
) -> JsResult<Local<'s>> {
    Ok(Local::undefined())
}
