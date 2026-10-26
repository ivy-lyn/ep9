//! The verify after a GC (a checked mark plus a checked sweep, M4-P1) against [`Heap::verify`]:
//! each planted fault must fail both. And the E2 check (`e2_check_init`).
//!
//! ```text
//! plant fault --> heap.verify()            --> Err, message M
//!             --> GC with verify_after_gc  --> panic "heap verifier failed after GC: ... M"
//! restore     --> both clean
//! ```

use core::ptr::NonNull;
use core::sync::atomic::Ordering::Relaxed;
use std::panic::{AssertUnwindSafe, catch_unwind};

use ep9_value::Value;

use super::*;
use crate::layout::{PAGE_SIZE, ZAP_WORD};
use crate::{E2Check, RootStrength, gc_state};

/// A full collection with precise roots only (the conservative scan reads no words).
fn precise_gc(heap: &Heap) {
    heap.collect_with_words(&[]);
}

/// The panic text of a GC that must fail its verify.
fn failed_gc(heap: &Heap) -> String {
    let r = catch_unwind(AssertUnwindSafe(|| precise_gc(heap)));
    let p = r.expect_err("the verify after the GC did not fail");
    let msg = p
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_default();
    assert!(msg.contains("heap verifier failed after GC"), "{msg}");
    msg
}

/// Both entries report a fault whose text contains `want`.
fn both_fail(heap: &Heap, want: &str) {
    let e = heap.verify().expect_err("Heap::verify missed the fault");
    assert!(e.to_string().contains(want), "Heap::verify: {e}");
    let m = failed_gc(heap);
    assert!(m.contains(want), "after GC: {m}");
}

/// Both entries are clean.
fn both_clean(heap: &Heap) {
    heap.verify().expect("Heap::verify after restore");
    precise_gc(heap);
}

fn checked_heap() -> Heap {
    test_heap(|c| c.verify_after_gc = true)
}

/// Writes raw bits into slot `i` of a live node.
fn raw(c: CellPtr, i: usize, bits: u64) {
    // SAFETY: slot i of the live node `c` is a Value slot; the test restores it.
    unsafe { c.field::<u64>(NODE_HEADER + 8 * i).write(bits) }
}

/// The planted-bug test (plan §3.2 task 3): a slot that points to a freed cell fails the
/// verify that runs after the next GC, with the slot's host named.
#[test]
fn a_planted_dangling_slot_fails_the_verify_after_gc() {
    let heap = checked_heap();
    let a = new_node(&heap, 1, 2);
    let g = heap.new_global(a.to_value());
    let b = new_node(&heap, 2, 0);
    let b_addr = b.addr();
    precise_gc(&heap); // b is unreachable: freed (the verify sweeps at once)
    assert!(!heap.is_cell(b_addr));
    raw(a, 1, b_addr as u64);
    let m = failed_gc(&heap);
    assert!(m.contains("not the start of a live cell"), "{m}");
    assert!(
        m.contains(&format!("{:#x}", a.addr())),
        "the host is named: {m}"
    );
    assert!(heap.verify().is_err());
    raw(a, 1, Value::UNDEFINED.to_bits());
    both_clean(&heap);
    heap.drop_global(g);
}

/// Every fault that `verifier_catches_corruption` (gc.rs) plants for `Heap::verify`, plus the
/// faults only the after-GC entry sees early, fails both entries.
#[test]
fn the_verify_after_gc_finds_what_verify_finds() {
    let heap = checked_heap();
    let a = new_node(&heap, 1, 2);
    let b = new_node(&heap, 2, 0);
    let g = heap.new_global(a.to_value());
    store(&heap, a, 0, b.to_value());
    both_clean(&heap);

    // Slot faults: an interior pointer, the bits 0, invalid Value bits.
    raw(a, 1, (b.addr() + 8) as u64);
    both_fail(&heap, "not the start of a live cell");
    raw(a, 1, 0);
    both_fail(&heap, "bits 0");
    raw(a, 1, ZAP_WORD);
    both_fail(&heap, "invalid Value bits");
    raw(a, 1, Value::UNDEFINED.to_bits());
    both_clean(&heap);

    // An unregistered kind byte on a live cell.
    // SAFETY: `b` is live; the test restores its kind.
    unsafe { b.header() }.set_kind(200);
    both_fail(&heap, "unregistered kind 200");
    // SAFETY: as above.
    unsafe { b.header() }.set_kind(NODE.get());
    both_clean(&heap);

    // A strong root that is not a cell start.
    let bad = heap.new_global(Value::UNDEFINED);
    // SAFETY: the bits form a cell pattern (8-aligned, below 2^47) that is not a live cell;
    // only the verifier and the checked mark read it.
    heap.set_global(bad, unsafe {
        Value::from_bits_unchecked((b.addr() + 16) as u64)
    });
    both_fail(&heap, "global handle");
    heap.drop_global(bad);
    both_clean(&heap);

    // A weak root range slot that points into a cell.
    let weak: Box<[Slot]> = Box::new([Slot::new(Value::UNDEFINED)]);
    // SAFETY: `weak` outlives the range (removed below); its slot holds Value bits.
    let id = unsafe { heap.add_root_range(NonNull::from(&weak[..]), RootStrength::Weak) };
    // SAFETY: as for `bad`.
    weak[0].gc_store(unsafe { Value::from_bits_unchecked((b.addr() + 16) as u64) });
    let e = heap
        .verify()
        .expect_err("Heap::verify missed the weak range fault");
    assert!(
        e.to_string().contains("not the start of a live cell"),
        "{e}"
    );
    let m = failed_gc(&heap);
    assert!(
        m.contains("weak range") && m.contains("not the start of a live cell"),
        "{m}"
    );
    weak[0].gc_store(Value::UNDEFINED);
    both_clean(&heap);
    assert!(heap.remove_root(id));

    // A mark bit outside a GC, on a granule with no cell.
    let stray = {
        let inner = heap.inner.borrow();
        let page = inner.mem.page(inner.mem.chunk_index(a.addr()).unwrap());
        let addr = page.addr() + PAGE_SIZE - 8;
        assert!(!page.vo(addr));
        assert!(page.mark(addr));
        addr
    };
    let clear_stray = || {
        let inner = heap.inner.borrow();
        let page = inner.mem.page(inner.mem.chunk_index(stray).unwrap());
        let (w, m) = page.bit(stray);
        let word = page.mark_word(w);
        word.store(word.load(Relaxed) & !m, Relaxed);
    };
    both_fail(&heap, "mark bit set outside a GC");
    clear_stray();
    both_clean(&heap);

    // A live cell whose gc_state is bad: the checked mark sets BLACK before anyone reads it
    // (as before M4-P1, where the verify ran after the mark), so only Heap::verify sees it.
    // SAFETY: `b` is live; the test restores it.
    unsafe { b.header() }.set_gc_state(9);
    assert!(
        heap.verify()
            .unwrap_err()
            .to_string()
            .contains("gc_state 9")
    );
    precise_gc(&heap);
    // SAFETY: as above.
    assert_eq!(unsafe { b.header() }.gc_state(), gc_state::BLACK);
    both_clean(&heap);
    heap.drop_global(g);
}

/// The E2 check (plan §3.2 task 4): an init store of a cell into a host that survived a GC is
/// a wrong E2 claim. Report mode counts it; assert mode panics. The mode is process-wide, so
/// one test covers both modes (no other test of this binary calls `e2_check_init`).
#[test]
fn the_e2_check_reports_and_asserts_a_wrong_claim() {
    // A GC-stress heap turns the check on in assert mode (since the M4-P1 fixer, V9-A20)
    // unless a mode is set; only this test sets one.
    let heap = test_heap(|c| c.gc_stress = 1);
    assert_eq!(crate::E2_CHECK_UNDER_GC_STRESS, E2Check::Assert);
    assert_eq!(crate::e2_check_mode(), E2Check::Assert);
    heap.set_gc_stress(0);

    let host = new_node(&heap, 1, 1);
    let g = heap.new_global(host.to_value());
    let young = new_node(&heap, 2, 0);
    let here = core::panic::Location::caller();
    // Before a GC the host is WHITE: the claim holds.
    crate::set_e2_check(E2Check::Assert);
    crate::e2_check_init(host, young.to_value().to_bits(), here);

    // After a GC the host is BLACK.
    store(&heap, host, 0, young.to_value());
    precise_gc(&heap);
    // SAFETY: `host` is rooted.
    assert_eq!(unsafe { host.header() }.gc_state(), gc_state::BLACK);
    let fresh = new_node(&heap, 3, 0);
    // A non-cell value needs no barrier: the claim is harmless.
    crate::e2_check_init(host, Value::int32(7).to_bits(), here);

    // Report mode: counted, no panic.
    crate::set_e2_check(E2Check::Report);
    let at = core::panic::Location::caller();
    crate::e2_check_init(host, fresh.to_value().to_bits(), at);
    crate::e2_check_init(host, fresh.to_value().to_bits(), at);
    let reports = crate::e2_reports();
    let line = at.to_string();
    assert!(
        reports.iter().any(|(l, n)| *l == line && *n == 2),
        "{reports:?}"
    );

    // Assert mode: the planted wrong E2 store panics and names its caller.
    crate::set_e2_check(E2Check::Assert);
    let r = catch_unwind(|| crate::e2_check_init(host, fresh.to_value().to_bits(), at));
    let p = r.expect_err("assert mode did not panic");
    let msg = p.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(msg.contains("E2 check") && msg.contains(&line), "{msg}");
    crate::set_e2_check(crate::E2_CHECK_UNDER_GC_STRESS);
    heap.drop_global(g);
}
