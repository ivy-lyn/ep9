//! Stack bounds on the process main thread. `harness = false`, so `main` runs on the main
//! thread (libtest runs `#[test]` functions on worker threads).

use ep9_platform::{current_thread_stack_bounds, stack_pointer};

fn main() {
    let b = current_thread_stack_bounds().unwrap();
    let local = 0u64;
    let here = std::hint::black_box(&local) as *const u64 as usize;
    let sp = stack_pointer();
    println!(
        "main thread: low {:#x} high {:#x} size {} KiB; sp {sp:#x}",
        b.low,
        b.high,
        b.size() >> 10
    );
    assert!(
        b.contains(here) && b.contains(sp),
        "main-thread bounds do not contain sp"
    );
    assert!(
        b.size() >= 256 << 10,
        "main-thread stack is smaller than 256 KiB"
    );
    println!("stack_main: ok");
}
