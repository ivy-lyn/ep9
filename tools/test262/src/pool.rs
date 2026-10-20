//! Parallel map over a slice on scoped threads with a sized stack.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Items each worker takes at a time. Small, so slow files spread out.
const CHUNK: usize = 8;

/// Run `f` on every item on `threads` threads, each with a `stack`-byte stack.
/// Returns the results in item order. `f` must not panic (catch panics inside it).
pub fn par_map<T, R, F>(items: &[T], threads: usize, stack: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let next = AtomicUsize::new(0);
    let threads = threads.clamp(1, items.len().max(1));
    let parts: Vec<Vec<(usize, R)>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|n| {
                std::thread::Builder::new()
                    .name(format!("ep9-test262-{n}"))
                    .stack_size(stack)
                    .spawn_scoped(scope, || {
                        let mut out = Vec::new();
                        loop {
                            let start = next.fetch_add(CHUNK, Ordering::Relaxed);
                            if start >= items.len() {
                                return out;
                            }
                            for (i, item) in items.iter().enumerate().skip(start).take(CHUNK) {
                                out.push((i, f(item)));
                            }
                        }
                    })
                    .expect("spawn a worker thread")
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().expect("worker thread panicked"))
            .collect()
    });
    let mut slots: Vec<Option<R>> = std::iter::repeat_with(|| None).take(items.len()).collect();
    for (i, r) in parts.into_iter().flatten() {
        slots[i] = Some(r);
    }
    slots
        .into_iter()
        .map(|r| r.expect("every item ran once"))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn keeps_order_and_covers_every_item() {
        let items: Vec<u32> = (0..1000).collect();
        let out = super::par_map(&items, 7, 1 << 20, |x| x * 2);
        assert_eq!(out, items.iter().map(|x| x * 2).collect::<Vec<_>>());
        assert!(super::par_map(&[] as &[u32], 4, 1 << 20, |x| *x).is_empty());
    }
}
