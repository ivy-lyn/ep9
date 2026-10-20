//! Stack bounds of the current thread.

use ep9_platform::{current_thread_stack_bounds, stack_pointer};

#[inline(never)]
fn deeper(n: u32) -> usize {
    let local = [n as u8; 64];
    let sp = stack_pointer();
    let addr = std::hint::black_box(&local).as_ptr() as usize;
    if n == 0 {
        sp.min(addr)
    } else {
        deeper(n - 1).min(sp)
    }
}

fn check_current_thread(min_size: usize) {
    let b = current_thread_stack_bounds().unwrap();
    let local = 0u64;
    let here = std::hint::black_box(&local) as *const u64 as usize;
    let sp = stack_pointer();
    let deep = deeper(50);
    println!(
        "{:?}: low {:#x} high {:#x} size {} KiB; sp {:#x}; deep sp {:#x}",
        std::thread::current().name(),
        b.low,
        b.high,
        b.size() >> 10,
        sp,
        deep
    );
    assert!(b.contains(here) && b.contains(sp) && b.contains(deep));
    assert!(deep < sp, "the stack grows down");
    assert!(b.size() >= min_size, "stack size {} < {min_size}", b.size());
}

#[test]
fn bounds_of_test_thread() {
    check_current_thread(64 << 10);
}

#[test]
fn bounds_of_spawned_thread() {
    const SIZE: usize = 4 << 20;
    std::thread::Builder::new()
        .name("ep9-stack-4MiB".into())
        .stack_size(SIZE)
        .spawn(|| check_current_thread(SIZE))
        .unwrap()
        .join()
        .unwrap();
}
