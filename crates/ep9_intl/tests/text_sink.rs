//! The polled drop of an Intl parts list after a stop (route INTL-TEXT-SINK; H23 "Interrupt
//! poll"): the drop polls in steps after the poll that stopped the work, and `polled` must
//! still give the stop. The text sink itself has unit tests in `number_format/decimal.rs`.

use ep9_intl::digits::{Interrupted, POLL_DIGITS, polled};
use ep9_vm::{Isolate, IsolateConfig};

/// A poll after the stop does not hide the stop: `polled` keeps the first `Err` (the
/// termination). The termination is requested before the call, so the poll that finds it
/// comes when the budget of 10,000 units runs out; the poll after it starts a new budget and
/// returns `Ok`. Old code: that `Ok` took the place of the stop, so `polled` reached its
/// "Interrupted after an Ok poll" `unreachable!` and panicked.
#[test]
fn a_poll_after_the_stop_keeps_the_stop() {
    let mut iso = Isolate::new(IsolateConfig {
        builtins: Some(&ep9_builtins::TABLE),
        ..IsolateConfig::default()
    })
    .expect("an isolate");
    iso.terminate_handle().terminate();
    let r = iso.with_cx(|cx| {
        polled(cx, |w| {
            let mut polls = 0usize;
            while w.add(POLL_DIGITS).is_ok() {
                polls += 1;
                // The budget is 10,000 units: the stop comes long before this bound.
                assert!(polls < 1_000_000, "no stop");
            }
            // The polled drop of a parts list after the stop.
            let _ = w.add(POLL_DIGITS);
            Err::<(), _>(Interrupted)
        })
    });
    assert!(r.is_err_and(|e| e.terminated), "the stop is kept");
}
