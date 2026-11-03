//! The short lazy-deopt probe (M6-P2 SPIKEC-10; plan v11 §4.8 task 1, the V11-D28 speed
//! branch; H29-Q4, SC R3): the return-address swap of one `Op::Call` frame under placement
//! (b), and a backtrace through the swapped frame.
//!
//! ```text
//!  config C: f compiled, its call records (deopt_meta::CallRecord: return offset, lazy exit)
//!  main -> f (compiled) --call_any--> g (T0) --> lazyDeopt() (host function; armed once):
//!     frame_returns()  (the backtrace before: the frame-record walk, ep9_vm::NativeCx)
//!     std::backtrace   (the system unwinder: CFI only, no FDE for JIT code)
//!     NativeCx::lazy_swap(f's code, its sites): the record whose return address is f's
//!        call return gets f's lazy exit
//!     frame_returns(), std::backtrace again (the backtrace through the swapped frame)
//!  g returns --> call_any returns into f's lazy exit --> shared exit --> the call record
//!     --> T0 runs f on after the call (acc = g's result) --> output equal to T0
//! ```
//!
//! What it proves and what it does not is H29-Q4's text (the gated part names the rest).

use std::cell::RefCell;
use std::ops::Range;

use ep9_vm::{Args, Cx, JsResult, Local};

/// The probe program (`spike-c lazy`, `tier::lazy_tests`): `f` keeps an int32 (`s`), a
/// double (`t`) and a parameter (`a`) live across its call of `g`, and `g` calls the hook.
pub const DEFS: &str = "\
function g(x) { lazyDeopt(); return x * 2 + 0.5; }\n\
function f(a, b) { var s = a + 1; var t = b * 1.5; var r = g(s); return r + s + t + a; }\n";
/// The warm-up of [`DEFS`] (the feedback `f` is compiled from).
pub const WARM: &str = "for (var w = 0; w < 30; w++) f(w, w * 0.5 + 0.25);";
/// The main script: the first call is swapped, the next two run the compiled code.
pub const MAIN: &str = "print(f(3, 2.5)); print(f(4, 1.25)); print(f(5, 0.5));";

/// What one armed `lazyDeopt()` call saw and did.
#[derive(Clone, Debug, Default)]
pub struct Swap {
    /// Each swapped frame record: its address and its old return address.
    pub swapped: Vec<(usize, usize)>,
    /// The frame-record walk's return addresses before and after the swap.
    pub before: Vec<usize>,
    pub after: Vec<usize>,
    /// The frames `std::backtrace` printed before and after the swap, and whether it
    /// reached [`crate::tier::lazy_probe`], a frame outside the compiled code's call (the
    /// system unwinder went through the compiled frame).
    pub std_before: (usize, bool),
    pub std_after: (usize, bool),
}

struct Armed {
    code: Range<usize>,
    sites: Vec<(u32, u32)>,
    done: Option<Swap>,
}

thread_local! {
    static ARMED: RefCell<Option<Armed>> = const { RefCell::new(None) };
}

/// Arms this thread's next `lazyDeopt()` call: it swaps the frames of the code at `code`
/// whose return addresses are one of `sites` (return offset, lazy exit offset).
pub fn arm(code: Range<usize>, sites: Vec<(u32, u32)>) {
    ARMED.with(|a| {
        *a.borrow_mut() = Some(Armed {
            code,
            sites,
            done: None,
        });
    });
}

/// Disarms, and gives what the armed call did (`None`: no `lazyDeopt()` ran while armed).
pub fn take() -> Option<Swap> {
    ARMED.with(|a| a.borrow_mut().take().and_then(|a| a.done))
}

/// The frames of a `std::backtrace` taken here (its numbered lines), and whether one of
/// them is `tier::lazy_probe` (its symbol line names it as `lazy_probe`).
fn std_frames() -> (usize, bool) {
    let text = std::backtrace::Backtrace::force_capture().to_string();
    let n = text
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.split_once(':')
                .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .count();
    (n, text.contains("lazy_probe"))
}

/// `lazyDeopt()`: does nothing unless armed ([`arm`]); armed, the first call walks the frame
/// records, swaps, and walks again (the backtrace through the swapped frame). Every session
/// has it (`harness::Session`), so T0 and config C run the same script.
pub(crate) fn host_lazy_deopt<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let job = ARMED.with(|a| {
        a.borrow()
            .as_ref()
            .filter(|a| a.done.is_none())
            .map(|a| (a.code.clone(), a.sites.clone()))
    });
    if let Some((code, sites)) = job {
        let before = ep9_vm::NativeCx::frame_returns();
        let std_before = std_frames();
        // SAFETY: `code` is the installed code whose call records name `sites` (`arm`'s
        // caller, `tier::lazy_probe`), and this host call is nested in the calls whose frames
        // it swaps, none of which has returned.
        let swapped = unsafe { ep9_vm::NativeCx::lazy_swap(code, &sites) };
        let after = ep9_vm::NativeCx::frame_returns();
        let std_after = std_frames();
        ARMED.with(|a| {
            if let Some(a) = a.borrow_mut().as_mut() {
                a.done = Some(Swap {
                    swapped,
                    before,
                    after,
                    std_before,
                    std_after,
                });
            }
        });
    }
    Ok(Local::undefined())
}
