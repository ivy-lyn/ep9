//! The tag parser checks the subtag syntax (1-8 ASCII letters or digits) on the input text
//! before it makes any subtag list, so a long invalid tag is rejected with no heap
//! allocation (Codex r1 CORE-2). Before, each subtag was an owned String first:
//! `"-".repeat(5e7)` took 1.66 GB at its peak.
//!
//! A counting global allocator (this test binary only) counts the bytes that the current
//! thread allocates.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountingAlloc;

thread_local! {
    static BYTES: Cell<u64> = const { Cell::new(0) };
}

fn count(size: usize) {
    // `try_with`: no count while the thread-local is being destroyed.
    let _ = BYTES.try_with(|b| b.set(b.get() + size as u64));
}

// SAFETY: every method forwards to `System` with the same arguments, so `System`'s contract
// holds; the count is a thread-local `Cell` with no destructor and allocates nothing.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: the caller keeps `GlobalAlloc::alloc`'s contract (non-zero size).
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: as `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from this allocator, so from `System`, with `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        // SAFETY: as `dealloc`, and the caller keeps `realloc`'s size contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static COUNTING_ALLOC: CountingAlloc = CountingAlloc;

/// The bytes that `f` allocates on this thread.
fn bytes_of(f: impl FnOnce() -> bool) -> (bool, u64) {
    let before = BYTES.with(Cell::get);
    let r = f();
    (r, BYTES.with(Cell::get) - before)
}

#[test]
fn an_invalid_tag_is_rejected_with_no_allocation() {
    let empty_subtags = "-".repeat(1_000_000);
    let long_last = format!("en{}-abcdefghi", "-a1".repeat(500_000));
    let not_ascii_last = format!("EN{}-\u{e9}", "-Ab".repeat(500_000));
    for tag in [&empty_subtags, &long_last, &not_ascii_last] {
        let (valid, bytes) = bytes_of(|| ep9_intl::tag::is_structurally_valid_language_tag(tag));
        assert!(!valid, "{}", &tag[..8]);
        assert_eq!(bytes, 0, "{}", &tag[..8]);
    }
    // A valid tag still parses (the control: the count sees the parser's lists).
    let (valid, bytes) =
        bytes_of(|| ep9_intl::tag::is_structurally_valid_language_tag("de-Latn-DE-u-co-phonebk"));
    assert!(valid);
    assert!(bytes > 0);
}
