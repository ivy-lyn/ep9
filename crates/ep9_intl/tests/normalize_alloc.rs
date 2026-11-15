//! W4 Codex r1 F2: `ep9_intl::normalize::normalize` stops at its `max` before it grows a
//! buffer past it. Before, the full ICU4X result and then a copy of it were made first:
//! `"\u{FDFA}".repeat(3e7).normalize("NFKC")` made two buffers of 1 GB or more before the
//! RangeError.
//!
//! A counting global allocator (this test binary only) counts the bytes that the current
//! thread allocates.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use ep9_intl::normalize::{Form, TooLong, normalize};

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

#[test]
fn no_buffer_past_the_limit() {
    // 100,000 x U+FDFA (200 KB): 1.8 million units (3.6 MB) in NFKC.
    let s: Vec<u16> = "\u{FDFA}".repeat(100_000).encode_utf16().collect();
    const MAX: usize = 50_000;
    let before = BYTES.with(Cell::get);
    let r = normalize(&s, Form::Nfkc, MAX);
    let bytes = BYTES.with(Cell::get) - before;
    assert_eq!(r, Err(TooLong));
    // One buffer of at most MAX units (2 bytes each), plus small ICU4X buffers.
    assert!(bytes < 4 * MAX as u64 * 2, "{bytes} bytes");
}
