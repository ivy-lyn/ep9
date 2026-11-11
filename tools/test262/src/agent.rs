//! `$262.detachArrayBuffer` and `$262.agent` of `ep9-test262 exec` (INTERPRETING.md
//! "Host-Defined Functions"; TypedArray TA-2 TA-HARNESS, design
//! `docs/plan/notes/typedarray-design.md` §5.6 and §8.2).
//!
//! ```text
//! worker run thread: the main agent ([[CanBlock]] true)        agent thread i ([[CanBlock]] true)
//!   run: a Session (deadline = now + the soft timeout)
//!   $262.agent.start(src) ──────── spawn ────────────────────►  a new Isolate (same config)
//!     returns when the agent runs (or ended) ◄──── running ───    agent prelude, then src
//!   $262.agent.broadcast(sab, n) ── a message per live agent ─►  $262.agent.receiveBroadcast(f)
//!     returns when each live agent took it                         waits, then f(sab', n): sab'
//!                                                                  is a SharedArrayBuffer of the
//!                                                                  agent on the same block
//!   $262.agent.getReport() ◄──────── the report queue ────────── $262.agent.report(v)
//!   $262.agent.sleep(ms), monotonicNow()                        sleep, monotonicNow, leaving
//!   the run ends (pass, fail, throw, timeout or panic):          microtask checkpoints until
//!     close the session, terminate each agent, wait up to          leaving() or the close
//!     STOP_GRACE for each one to end; one that does not end:
//!     an infrastructure error "timeout" (its thread is left)
//! ```
//!
//! | Rule | Statement |
//! |---|---|
//! | Deadline and kill path | Every wait of a hook (start, broadcast, receiveBroadcast, sleep) ends at the run's deadline or at the close of the session, and then throws (a host error, not a test result: [`run`] makes the run an infrastructure error "timeout"). The end of a run terminates every agent isolate (`TerminateHandle`) and waits [`STOP_GRACE`] for it. So no hang can pass a test |
//! | GC (plan rule 8) | Rust memory holds no JS value: the session holds strings, numbers, SharedArrayBuffer blocks (`ep9_vm::SharedBlockRef`: bytes, no JS value), thread handles and terminate handles. A hook keeps a value only in its arguments or in locals during one call |
//! | Engine checks | The hooks test no class: the brand-checked engine method gives the TypeError. `$262.detachArrayBuffer` of a value that is not an engine ArrayBuffer (`Cx::detach_array_buffer`); `$262.agent.broadcast` of a value that is not an engine SharedArrayBuffer (`Cx::shared_array_buffer_block`, [`share_block`]). An agent makes its SharedArrayBuffer on the block with the prototype of its realm's intrinsic `%SharedArrayBuffer.prototype%` ([`adopt_block`]; H22-Q8) |
//! | `[[CanBlock]]` | The main agent can block ([`MAIN_CAN_BLOCK`], as a shell main thread in d8, the SpiderMonkey shell and node: test262 `atomicsHelper.js` `$262.agent.safeBroadcast` calls `Atomics.wait` on it); agents can too. A `CanBlockIsFalse` test fails with the cause [`CAN_BLOCK_CAUSE`]; a `CanBlockIsTrue` test runs. The engine has no `[[CanBlock]]` input yet: `Atomics.wait` (TypedArray TA-4) reads it, and the runner sets it to true for the main isolate and for each agent isolate |
//!
//! Test hooks (`EP9_TEST262_FAULT`, see `exec::fault_for`):
//! - `agent-stuck:<substr>` makes each agent of a matching test sleep in Rust for
//!   [`STUCK_FOR`], deaf to its termination, so the tests can show that an agent that does
//!   not stop is an infrastructure error.
//! - `agent-hold-error:<substr>`: when the script or a job of an agent of a matching test
//!   ends with an exception that is not a termination, the agent waits in Rust until the
//!   session closes (or its deadline passes) before it decides if the exception came from
//!   the stop. So the tests can show that an agent error thrown before the close is kept.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ep9_frontend::{SourceOpts, Strictness};
use ep9_vm::{
    Args, Cx, ErrorKind, ExecOptions, HostFn, Isolate, JsResult, Local, TerminateHandle, Throw,
};

use crate::exec::{self, Run, RunCtx};
use crate::frontmatter::Meta;
use crate::parse::Mode;

/// The `[[CanBlock]]` of the main agent, the one that runs the test file. The main agent can
/// block (a shell main thread: test262 `atomicsHelper.js` `safeBroadcast` calls `Atomics.wait`
/// on it). `Atomics.wait` wakes on the isolate interrupt (design §5.6), so the watchdog stops
/// it.
pub(crate) const MAIN_CAN_BLOCK: bool = true;

/// The failure cause of a test whose `[[CanBlock]]` flag the main agent does not have (a
/// `CanBlockIsFalse` test).
pub(crate) const CAN_BLOCK_CAUSE: &str = "CanBlockIsFalse: the main agent can block";

/// The time an agent gets to end after the run ends and terminates it.
pub(crate) const STOP_GRACE: Duration = Duration::from_secs(5);

/// The sleep of the `agent-stuck` test hook (longer than [`STOP_GRACE`]).
const STUCK_FOR: Duration = Duration::from_secs(60);

/// The agents one run may start (test262 starts at most 4).
const MAX_AGENTS: usize = 16;

/// How often an idle agent runs its microtask checkpoint while it waits for `leaving()`.
const IDLE_TICK: Duration = Duration::from_millis(2);

/// The longest `$262.agent.sleep` outside a session (the unit tests of `exec`).
const MAX_SLEEP: Duration = Duration::from_secs(10);

/// The number of host functions of a test isolate: 4 of `exec`, then [`AGENT_FUNCTIONS`].
pub(crate) const HOST_COUNT: usize = 4 + AGENT_FUNCTIONS.len();

/// The host function list of a test isolate (`exec::HOST_FUNCTIONS`).
pub(crate) type HostList = [(&'static str, u32, HostFn); HOST_COUNT];

/// The global names of the `$262.detachArrayBuffer` and `$262.agent` functions. Every isolate
/// of a run (the main agent and each agent) gets all of them; a hook of the other side throws
/// a TypeError.
const AGENT_FUNCTIONS: [(&str, u32, HostFn); 9] = [
    ("__ep9_detach_array_buffer", 1, detach_array_buffer),
    ("__ep9_agent_start", 1, start),
    ("__ep9_agent_broadcast", 2, broadcast),
    ("__ep9_agent_get_report", 0, get_report),
    ("__ep9_agent_sleep", 1, sleep),
    ("__ep9_agent_monotonic_now", 0, monotonic_now),
    ("__ep9_agent_receive_broadcast", 1, receive_broadcast),
    ("__ep9_agent_report", 1, report),
    ("__ep9_agent_leaving", 0, leaving),
];

/// `base` (the 4 host functions of `exec`), then [`AGENT_FUNCTIONS`].
pub(crate) const fn host_functions(base: [(&'static str, u32, HostFn); 4]) -> HostList {
    let mut out = [base[0]; HOST_COUNT];
    let mut i = 0;
    while i < HOST_COUNT {
        out[i] = if i < 4 {
            base[i]
        } else {
            AGENT_FUNCTIONS[i - 4]
        };
        i += 1;
    }
    out
}

/// The `$262` additions of the main agent, after `exec::PRELUDE`.
const MAIN_TAIL: &str = "$262.detachArrayBuffer = __ep9_detach_array_buffer;\n\
    $262.agent = { start: __ep9_agent_start, broadcast: __ep9_agent_broadcast, \
    getReport: __ep9_agent_get_report, sleep: __ep9_agent_sleep, \
    monotonicNow: __ep9_agent_monotonic_now };\n";

/// The `$262` additions of an agent, after `exec::PRELUDE` (INTERPRETING.md: the agent's
/// `$262.agent` has `receiveBroadcast`, `report`, `sleep`, `leaving` and `monotonicNow`).
const AGENT_TAIL: &str = "$262.detachArrayBuffer = __ep9_detach_array_buffer;\n\
    $262.agent = { receiveBroadcast: __ep9_agent_receive_broadcast, \
    report: __ep9_agent_report, sleep: __ep9_agent_sleep, leaving: __ep9_agent_leaving, \
    monotonicNow: __ep9_agent_monotonic_now };\n";

/// The `$262` prelude of the isolate on this thread: the main agent's, or an agent's on an
/// agent thread. `$262.createRealm` runs it in the new realm too.
pub(crate) fn prelude() -> &'static str {
    static MAIN: OnceLock<String> = OnceLock::new();
    static AGENT: OnceLock<String> = OnceLock::new();
    let (cell, tail) = if is_agent_thread() {
        (&AGENT, AGENT_TAIL)
    } else {
        (&MAIN, MAIN_TAIL)
    };
    cell.get_or_init(|| format!("{}{tail}", exec::PRELUDE))
}

// ---- the session of one run ------------------------------------------------------------------

/// What an agent isolate is made with: the config of the main agent's isolate.
#[derive(Clone, Copy)]
struct AgentConfig {
    gc_stress: u32,
    feedback_threshold: u32,
    exec: ExecOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// The thread makes its isolate.
    Starting,
    /// The isolate exists; its terminate handle is in [`AgentState::terminate`].
    Running,
    /// The thread is done with the isolate (or never made one).
    Ended,
}

/// The second value of a broadcast (INTERPRETING.md: an Int32 or a BigInt).
#[derive(Clone, Debug)]
enum Payload {
    Undefined,
    Number(f64),
    /// The decimal digits of a BigInt.
    BigInt(String),
}

/// The block of a SharedArrayBuffer as a broadcast gives it to an agent: a `Send` handle that
/// keeps the block alive until the agent makes its SharedArrayBuffer on it (H22-Q8: then one
/// `Arc` clone per SharedArrayBuffer object, in the table of its isolate). It holds no JS
/// value.
type Block = ep9_vm::SharedBlockRef;

#[derive(Clone, Debug)]
struct Message {
    block: Block,
    value: Payload,
}

#[derive(Debug)]
struct AgentState {
    phase: Phase,
    inbox: VecDeque<Message>,
    /// Messages put into the inbox, and messages taken from it.
    sent: u64,
    taken: u64,
    /// `$262.agent.leaving()` ran.
    left: bool,
    /// An uncaught exception of the agent (not a termination): its text.
    error: Option<String>,
    /// A panic of the agent thread: its text.
    panic: Option<String>,
    terminate: Option<TerminateHandle>,
    /// A hook of this agent threw because the session closed ([`stopped`]): an exception that
    /// ends the agent after it is part of the stop. Set on the agent thread when the hook
    /// throws, so an exception thrown before the close is never taken as the stop.
    saw_stop: bool,
}

impl AgentState {
    fn new() -> AgentState {
        AgentState {
            phase: Phase::Starting,
            inbox: VecDeque::new(),
            sent: 0,
            taken: 0,
            left: false,
            error: None,
            panic: None,
            terminate: None,
            saw_stop: false,
        }
    }
}

#[derive(Debug, Default)]
struct State {
    /// The run ended: every wait ends, every agent stops.
    closed: bool,
    reports: VecDeque<String>,
    agents: Vec<AgentState>,
}

/// The state that the main agent and its agents share. One lock and one condition variable:
/// every change notifies all waiters (a run has few agents).
#[derive(Debug)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    deadline: Instant,
}

/// Why [`Shared::wait`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wait {
    Done,
    Closed,
    Deadline,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn notify(&self) {
        self.changed.notify_all();
    }

    /// Waits until `done` holds, the session closes (when `on_close`) or `until` passes.
    fn wait<'a>(
        &self,
        mut g: MutexGuard<'a, State>,
        until: Instant,
        on_close: bool,
        done: impl Fn(&State) -> bool,
    ) -> (MutexGuard<'a, State>, Wait) {
        loop {
            if done(&g) {
                return (g, Wait::Done);
            }
            if on_close && g.closed {
                return (g, Wait::Closed);
            }
            let now = Instant::now();
            if now >= until {
                return (g, Wait::Deadline);
            }
            g = self
                .changed
                .wait_timeout(g, until - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// The run state of the main agent (the thread-local [`ROLE`] of the run thread).
struct MainState {
    shared: Arc<Shared>,
    config: AgentConfig,
    threads: Vec<JoinHandle<()>>,
    /// The agent test hooks set for this test.
    hooks: Hooks,
    /// A hook threw because the run passed its deadline.
    deadline_hit: bool,
}

/// The agent test hooks of a test (module doc).
#[derive(Clone, Copy, Debug, Default)]
struct Hooks {
    /// `agent-stuck`.
    stuck: bool,
    /// `agent-hold-error`.
    hold_error: bool,
}

impl Hooks {
    fn for_test(rel: &str) -> Hooks {
        Hooks {
            stuck: exec::fault_for("agent-stuck", rel),
            hold_error: exec::fault_for("agent-hold-error", rel),
        }
    }
}

enum Role {
    /// No run (the unit tests of `exec`, the parent process).
    None,
    Main(MainState),
    Agent {
        shared: Arc<Shared>,
        index: usize,
    },
}

thread_local! {
    static ROLE: RefCell<Role> = const { RefCell::new(Role::None) };
}

fn is_agent_thread() -> bool {
    ROLE.with(|r| matches!(*r.borrow(), Role::Agent { .. }))
}

/// What the agents of a run left behind.
#[derive(Debug, Default)]
struct End {
    /// Agents still running [`STOP_GRACE`] after the end of the run.
    stuck: usize,
    errors: Vec<String>,
    panics: Vec<String>,
    deadline_hit: bool,
}

/// The agents of one run: [`Session::begin`] makes the main agent's state; [`Session::end`]
/// (or the drop, after a panic) stops every agent.
struct Session {
    ended: bool,
}

impl Session {
    fn begin(config: AgentConfig, timeout: Duration, hooks: Hooks) -> Session {
        let shared = Arc::new(Shared {
            state: Mutex::default(),
            changed: Condvar::new(),
            deadline: Instant::now() + timeout,
        });
        ROLE.with(|r| {
            *r.borrow_mut() = Role::Main(MainState {
                shared,
                config,
                threads: Vec::new(),
                hooks,
                deadline_hit: false,
            });
        });
        Session { ended: false }
    }

    fn end(mut self) -> End {
        self.ended = true;
        shut_down()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.ended {
            shut_down();
        }
    }
}

/// Closes the session of this thread: every wait ends, every agent isolate is terminated;
/// then waits up to [`STOP_GRACE`] for the agents to end. An agent thread that is still
/// running then is left (it is counted in [`End::stuck`]).
fn shut_down() -> End {
    let role = ROLE.with(|r| core::mem::replace(&mut *r.borrow_mut(), Role::None));
    let Role::Main(main) = role else {
        return End::default();
    };
    let shared = main.shared;
    {
        let mut g = shared.lock();
        g.closed = true;
        for a in &g.agents {
            if let Some(t) = &a.terminate {
                t.terminate();
            }
        }
    }
    shared.notify();
    let g = shared.lock();
    let until = Instant::now() + STOP_GRACE;
    let (g, _) = shared.wait(g, until, false, |s| {
        s.agents.iter().all(|a| a.phase == Phase::Ended)
    });
    let end = End {
        stuck: g.agents.iter().filter(|a| a.phase != Phase::Ended).count(),
        errors: g.agents.iter().filter_map(|a| a.error.clone()).collect(),
        panics: g.agents.iter().filter_map(|a| a.panic.clone()).collect(),
        deadline_hit: main.deadline_hit,
    };
    drop(g);
    for t in main.threads {
        // An ended agent returns at once; a stuck one is left (dropping the handle detaches it).
        if t.is_finished() || end.stuck == 0 {
            let _ = t.join();
        }
    }
    end
}

/// Runs one mode of a test (`RunCtx::run`) with the `[[CanBlock]]` rule and the agents of
/// the run: an agent that does not stop, a hook that waits past the deadline or an agent
/// panic is an infrastructure error; an uncaught exception of an agent fails a passing run.
pub(crate) fn run(cx: &mut RunCtx<'_>, rel: &str, src: &str, meta: &Meta, mode: Mode) -> Run {
    if let Some(want) = meta.can_block()
        && want != MAIN_CAN_BLOCK
    {
        return exec::fail(
            CAN_BLOCK_CAUSE,
            "the test needs [[CanBlock]] false; the runner runs it in the main agent, whose \
             [[CanBlock]] is true (a shell main thread can block)",
        );
    }
    let opts = cx.opts;
    let config = AgentConfig {
        gc_stress: opts.gc_stress,
        feedback_threshold: opts.feedback_threshold,
        exec: opts.exec,
    };
    let session = Session::begin(config, opts.timeout, Hooks::for_test(rel));
    let run = cx.run(rel, src, meta, mode);
    merge(run, &session.end())
}

/// The result of a run whose agents left `end`.
fn merge(run: Run, end: &End) -> Run {
    if matches!(run, Run::Infra { .. }) {
        return run;
    }
    if let Some(p) = end.panics.first() {
        return exec::infra("panic", format!("panic in an agent: {p}"));
    }
    if end.stuck > 0 {
        return exec::infra(
            "timeout",
            format!(
                "timeout: {} agent(s) still ran {STOP_GRACE:?} after the run ended and was \
                 terminated",
                end.stuck
            ),
        );
    }
    if end.deadline_hit {
        return exec::infra(
            "timeout",
            "timeout: a $262.agent wait passed the run's deadline",
        );
    }
    let Some(first) = end.errors.first() else {
        return run;
    };
    let agents = format!("agent errors: {}", end.errors.join("; "));
    match run {
        Run::Pass => exec::fail(
            format!("agent threw: {}", exec::cause_of_message(first)),
            agents,
        ),
        Run::Fail { cause, detail } => exec::fail(cause, format!("{detail}; {agents}")),
        infra @ Run::Infra { .. } => infra,
    }
}

// ---- the agent thread --------------------------------------------------------------------------

fn agent_thread(shared: &Arc<Shared>, index: usize, config: AgentConfig, src: &str, hooks: Hooks) {
    ROLE.with(|r| {
        *r.borrow_mut() = Role::Agent {
            shared: Arc::clone(shared),
            index,
        };
    });
    let r = catch_unwind(AssertUnwindSafe(|| {
        agent_body(shared, index, config, src, hooks)
    }));
    ROLE.with(|r| *r.borrow_mut() = Role::None);
    let mut g = shared.lock();
    let a = &mut g.agents[index];
    a.phase = Phase::Ended;
    a.terminate = None;
    match r {
        Ok(Ok(())) => {}
        Ok(Err(e)) => a.error = Some(format!("agent {index}: {e}")),
        Err(_) => a.panic = Some(format!("agent {index}: {}", exec::take_panic())),
    }
    drop(g);
    shared.notify();
}

/// The life of agent `index`: its isolate, the agent prelude, its script, then microtask
/// checkpoints until it calls `leaving()`, the session closes or the deadline passes. `Err` =
/// an uncaught exception (not a termination) or a script that does not compile.
fn agent_body(
    shared: &Shared,
    index: usize,
    config: AgentConfig,
    src: &str,
    hooks: Hooks,
) -> Result<(), String> {
    let name = format!("agent {index}");
    let unit = exec::compile(src, SourceOpts::script(Strictness::Sloppy), &name)
        .map_err(|e| format!("the script does not compile: {e}"))?;
    let iso_config = exec::isolate_config(config.gc_stress, config.feedback_threshold);
    let mut iso = Isolate::new_with(iso_config, config.exec)
        .map_err(|e| format!("cannot create an isolate: {e}"))?;
    for (n, length, f) in exec::HOST_FUNCTIONS {
        iso.register_host_function(n, length, f);
    }
    {
        let mut g = shared.lock();
        let handle = iso.terminate_handle();
        if g.closed {
            handle.terminate();
        }
        let a = &mut g.agents[index];
        a.terminate = Some(handle);
        a.phase = Phase::Running;
    }
    shared.notify();
    if hooks.stuck {
        // The `agent-stuck` test hook: deaf to the termination.
        std::thread::sleep(STUCK_FOR);
        return Ok(());
    }
    // An exception is the stop if it is the termination or comes after a hook of this agent
    // threw for the close (`saw_stop`), not because the session is closed by now: an agent
    // error thrown before the close stays an error, although the close comes first.
    let stopped = |e: &ep9_vm::Exception| {
        if hooks.hold_error && !e.terminated {
            // The `agent-hold-error` test hook: the close comes before the decision.
            let g = shared.lock();
            drop(shared.wait(g, shared.deadline, true, |_| false));
        }
        e.terminated || shared.lock().agents[index].saw_stop
    };
    let r = iso.with_cx_checkpoint(|cx| {
        cx.evaluate_script(prelude(), "harness/$262 agent prelude")?;
        cx.run_script(unit, &name).map(|_| ())
    });
    match r {
        Ok(()) => {}
        Err(e) if stopped(&e) => return Ok(()),
        Err(e) => return Err(e.message),
    }
    loop {
        {
            let g = shared.lock();
            if g.closed || g.agents[index].left || Instant::now() >= shared.deadline {
                return Ok(());
            }
        }
        match iso.perform_microtask_checkpoint() {
            Ok(()) => {}
            Err(e) if stopped(&e) => return Ok(()),
            Err(e) => return Err(e.message),
        }
        let g = shared.lock();
        let until = Instant::now() + IDLE_TICK;
        drop(shared.wait(g, until, true, |s| s.agents[index].left));
    }
}

// ---- the hooks ---------------------------------------------------------------------------------

/// The session of the main agent, or a TypeError on another thread.
fn main_shared(cx: &mut Cx<'_>, hook: &str) -> JsResult<Arc<Shared>> {
    let s = ROLE.with(|r| match &*r.borrow() {
        Role::Main(m) => Some(Arc::clone(&m.shared)),
        _ => None,
    });
    s.ok_or_else(|| {
        cx.throw_type_error(&format!(
            "$262.agent.{hook}: only the main agent of a test run has it"
        ))
    })
}

/// The session and index of this agent, or a TypeError on another thread.
fn agent_shared(cx: &mut Cx<'_>, hook: &str) -> JsResult<(Arc<Shared>, usize)> {
    let s = ROLE.with(|r| match &*r.borrow() {
        Role::Agent { shared, index } => Some((Arc::clone(shared), *index)),
        _ => None,
    });
    s.ok_or_else(|| {
        cx.throw_type_error(&format!(
            "$262.agent.{hook}: only an agent of $262.agent.start has it"
        ))
    })
}

/// The error of a hook whose wait ended at the deadline or at the close of the session. At the
/// deadline the main agent's run becomes an infrastructure error ([`merge`]).
fn stopped(cx: &mut Cx<'_>, hook: &str, why: Wait) -> Throw {
    if why == Wait::Closed {
        // Codex TA-2 r1 finding 1: only this error (and what JS makes of it) is the stop.
        ROLE.with(|r| {
            if let Role::Agent { shared, index } = &*r.borrow() {
                shared.lock().agents[*index].saw_stop = true;
            }
        });
    }
    if why == Wait::Deadline {
        ROLE.with(|r| {
            if let Role::Main(m) = &mut *r.borrow_mut() {
                m.deadline_hit = true;
            }
        });
    }
    let what = if why == Wait::Deadline {
        "the run passed its deadline"
    } else {
        "the run ended"
    };
    cx.throw_error(ErrorKind::Error, &format!("$262.agent.{hook}: {what}"))
}

/// ToString(`v`) as Rust text (a lone surrogate is a TypeError).
fn source_text<'s>(cx: &mut Cx<'s>, v: Local<'s>, hook: &str) -> JsResult<String> {
    let s = cx.to_string(v)?;
    let flat = cx.flatten(s)?;
    let units: Vec<u16> = (0..cx.string_len(s))
        .map(|i| cx.code_unit_at(flat, i).unwrap_or(0))
        .collect();
    String::from_utf16(&units).map_err(|_| {
        cx.throw_type_error(&format!(
            "$262.agent.{hook}: the source text has a lone surrogate"
        ))
    })
}

/// `$262.detachArrayBuffer(buffer)`: DetachArrayBuffer(buffer) with key undefined
/// (sec-detacharraybuffer), through the engine's detach (`Cx::detach_array_buffer`): every
/// view of the buffer gets length 0. A TypeError if `buffer` is not an ArrayBuffer of the
/// engine (a SharedArrayBuffer included) or its detach key is not undefined.
fn detach_array_buffer<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    cx.detach_array_buffer(args.get(0), Local::undefined())?;
    Ok(Local::undefined())
}

/// `$262.agent.start(source)` (main agent): a new agent (a new isolate on its own thread)
/// runs ToString(`source`) as a script; returns when the agent runs or ended.
fn start<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let shared = main_shared(cx, "start")?;
    let src = source_text(cx, args.get(0), "start")?;
    if Instant::now() >= shared.deadline {
        return Err(stopped(cx, "start", Wait::Deadline));
    }
    let index = {
        let mut g = shared.lock();
        if g.agents.len() >= MAX_AGENTS {
            None
        } else {
            g.agents.push(AgentState::new());
            Some(g.agents.len() - 1)
        }
    };
    let Some(index) = index else {
        return Err(cx.throw_range_error(&format!(
            "$262.agent.start: a run starts at most {MAX_AGENTS} agents"
        )));
    };
    let (config, hooks) = ROLE.with(|r| match &*r.borrow() {
        Role::Main(m) => (m.config, m.hooks),
        _ => unreachable!("main_shared checked the role"),
    });
    let s = Arc::clone(&shared);
    let spawned = std::thread::Builder::new()
        .name(format!("ep9-test262-agent-{index}"))
        .stack_size(exec::RUN_STACK_BYTES)
        .spawn(move || agent_thread(&s, index, config, &src, hooks));
    match spawned {
        Ok(t) => ROLE.with(|r| {
            if let Role::Main(m) = &mut *r.borrow_mut() {
                m.threads.push(t);
            }
        }),
        Err(e) => {
            shared.lock().agents[index].phase = Phase::Ended;
            return Err(cx.throw_error(
                ErrorKind::Error,
                &format!("$262.agent.start: cannot start a thread: {e}"),
            ));
        }
    }
    let g = shared.lock();
    let (g, why) = shared.wait(g, shared.deadline, true, |s| {
        s.agents[index].phase != Phase::Starting
    });
    drop(g);
    match why {
        Wait::Done => Ok(Local::undefined()),
        w => Err(stopped(cx, "start", w)),
    }
}

/// The block of the SharedArrayBuffer `v` for a broadcast. A TypeError if `v` is not a
/// SharedArrayBuffer of the engine: the brand-checked `Cx::shared_array_buffer_block` gives it
/// (no class test here; design study judge 4.7 (1)).
fn share_block<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Block> {
    cx.shared_array_buffer_block(v)
}

/// A SharedArrayBuffer of the current realm of this agent on the block `b`: the same bytes,
/// the block's byte length now and its maximum. Its prototype is the realm's intrinsic
/// `%SharedArrayBuffer.prototype%`, not a global lookup (no user code runs, and a test that
/// changes the global `SharedArrayBuffer` does not change it).
fn adopt_block<'s>(cx: &mut Cx<'s>, b: &Block) -> JsResult<Local<'s>> {
    let id = ep9_vm::IntrinsicId::SharedArrayBufferPrototype;
    if !cx.has_intrinsic(id) {
        return Err(
            cx.throw_type_error("$262.agent.receiveBroadcast: this realm has no SharedArrayBuffer")
        );
    }
    let proto = cx.intrinsic(id);
    cx.new_shared_array_buffer_on_block(proto, b)
        .map(Into::into)
}

/// The second argument of a broadcast: undefined, a Number or a BigInt, else a TypeError.
fn payload<'s>(cx: &mut Cx<'s>, v: Local<'s>) -> JsResult<Payload> {
    if v.is_undefined() {
        return Ok(Payload::Undefined);
    }
    if let Some(n) = v.as_number() {
        return Ok(Payload::Number(n));
    }
    if cx.is_bigint(v) {
        let s = cx.bigint_to_string(v, 10)?;
        return cx.to_rust_string(s).map(Payload::BigInt);
    }
    Err(cx
        .throw_type_error("$262.agent.broadcast: the second argument is not a Number or a BigInt"))
}

/// The JS value of `p` in the current realm.
fn payload_value<'s>(cx: &mut Cx<'s>, p: &Payload) -> JsResult<Local<'s>> {
    match p {
        Payload::Undefined => Ok(Local::undefined()),
        Payload::Number(n) => Ok(Local::number(*n)),
        Payload::BigInt(digits) => {
            let s = cx.new_string(digits)?;
            cx.to_bigint(s)
        }
    }
}

/// `$262.agent.broadcast(sab, value)` (main agent): each live agent gets (the block of `sab`,
/// `value`); returns when each of them took it (or ended).
fn broadcast<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let shared = main_shared(cx, "broadcast")?;
    let block = share_block(cx, args.get(0))?;
    let value = payload(cx, args.get(1))?;
    let mut targets = Vec::new();
    {
        let mut g = shared.lock();
        for (i, a) in g.agents.iter_mut().enumerate() {
            if a.phase != Phase::Ended {
                a.inbox.push_back(Message {
                    block: block.clone(),
                    value: value.clone(),
                });
                a.sent += 1;
                targets.push((i, a.sent));
            }
        }
    }
    shared.notify();
    let g = shared.lock();
    let (g, why) = shared.wait(g, shared.deadline, true, |s| {
        targets.iter().all(|&(i, n)| {
            let a = &s.agents[i];
            a.taken >= n || a.phase == Phase::Ended
        })
    });
    drop(g);
    match why {
        Wait::Done => Ok(Local::undefined()),
        w => Err(stopped(cx, "broadcast", w)),
    }
}

/// `$262.agent.getReport()` (main agent): the oldest report as a string, or null. With no
/// report left and an agent that threw, an Error that names the agent's exception (so a test
/// that waits for a report fails with the cause instead of a timeout).
fn get_report<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let shared = main_shared(cx, "getReport")?;
    let next = {
        let mut g = shared.lock();
        match g.reports.pop_front() {
            Some(s) => Ok(s),
            None => Err(g.agents.iter().find_map(|a| a.error.clone())),
        }
    };
    match next {
        Ok(s) => Ok(cx.new_string(&s)?.into()),
        Err(None) => Ok(Local::null()),
        Err(Some(e)) => Err(cx.throw_error(
            ErrorKind::Error,
            &format!("$262.agent.getReport: an agent threw: {e}"),
        )),
    }
}

/// `$262.agent.sleep(ms)` (both sides): sleeps ToNumber(`ms`) milliseconds (none for NaN or
/// ms ≤ 0). The sleep ends at the run's deadline (then an Error), and an agent's also at the
/// close of the session.
fn sleep<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let ms = cx.to_number(args.get(0))?;
    let d = if ms.is_nan() || ms <= 0.0 {
        Duration::ZERO
    } else {
        Duration::try_from_secs_f64(ms / 1000.0).unwrap_or(Duration::MAX)
    };
    let side = ROLE.with(|r| match &*r.borrow() {
        Role::Main(m) => Some((Arc::clone(&m.shared), false)),
        Role::Agent { shared, .. } => Some((Arc::clone(shared), true)),
        Role::None => None,
    });
    let Some((shared, agent)) = side else {
        std::thread::sleep(d.min(MAX_SLEEP));
        return Ok(Local::undefined());
    };
    let until = Instant::now()
        .checked_add(d)
        .map_or(shared.deadline, |t| t.min(shared.deadline));
    let g = shared.lock();
    let (g, why) = shared.wait(g, until, agent, |_| false);
    drop(g);
    match why {
        Wait::Closed => Err(stopped(cx, "sleep", Wait::Closed)),
        _ if Instant::now() >= shared.deadline => Err(stopped(cx, "sleep", Wait::Deadline)),
        _ => Ok(Local::undefined()),
    }
}

/// `$262.agent.monotonicNow()` (both sides): milliseconds since a fixed point of this process,
/// from a monotonic clock (one clock for the main agent and every agent).
fn monotonic_now<'s>(
    _cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let origin = *ORIGIN.get_or_init(Instant::now);
    Ok(Local::number(origin.elapsed().as_secs_f64() * 1000.0))
}

/// `$262.agent.receiveBroadcast(f)` (agent): waits for the next broadcast, then calls
/// f(sab, value), where sab is a SharedArrayBuffer of this agent on the broadcast block. The
/// wait ends at the close of the session or the deadline (then an Error).
fn receive_broadcast<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let (shared, index) = agent_shared(cx, "receiveBroadcast")?;
    let f = args.get(0);
    if !cx.is_callable(f) {
        return Err(
            cx.throw_type_error("$262.agent.receiveBroadcast: the argument is not a function")
        );
    }
    let g = shared.lock();
    let (mut g, why) = shared.wait(g, shared.deadline, true, |s| {
        !s.agents[index].inbox.is_empty()
    });
    let msg = match why {
        Wait::Done => {
            let a = &mut g.agents[index];
            a.taken += 1;
            a.inbox.pop_front()
        }
        _ => None,
    };
    drop(g);
    shared.notify();
    let Some(msg) = msg else {
        return Err(stopped(cx, "receiveBroadcast", why));
    };
    let sab = adopt_block(cx, &msg.block)?;
    let value = payload_value(cx, &msg.value)?;
    cx.call(f, Local::undefined(), &[sab, value])?;
    Ok(Local::undefined())
}

/// `$262.agent.report(value)` (agent): ToString(`value`) joins the report queue.
fn report<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let (shared, _) = agent_shared(cx, "report")?;
    let s = cx.to_rust_string(args.get(0))?;
    shared.lock().reports.push_back(s);
    shared.notify();
    Ok(Local::undefined())
}

/// `$262.agent.leaving()` (agent): the agent is done; it ends after its current job.
fn leaving<'s>(
    cx: &mut Cx<'s>,
    _this: Local<'s>,
    _args: Args<'s>,
    _nt: Local<'s>,
) -> JsResult<Local<'s>> {
    let (shared, index) = agent_shared(cx, "leaving")?;
    shared.lock().agents[index].left = true;
    shared.notify();
    Ok(Local::undefined())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ep9_vm::PropertyDescriptor;

    /// A test isolate as `exec` makes one (`--gc-stress` = `gc_stress`), with the host
    /// functions of every test isolate.
    fn isolate(gc_stress: u32) -> Isolate {
        let mut iso = Isolate::new(exec::isolate_config(gc_stress, 0)).expect("an isolate");
        for (name, length, f) in exec::HOST_FUNCTIONS {
            iso.register_host_function(name, length, f);
        }
        iso
    }

    /// Defines the global `name` = `v`, so the GC sees `v` through the global object (H10).
    fn global<'s>(cx: &mut Cx<'s>, name: &str, v: Local<'s>) -> JsResult<()> {
        let g = cx.global();
        let desc = PropertyDescriptor::data(v, true, true, true);
        cx.define_property_or_throw(g, name, desc).map(|_| ())
    }

    /// `$262.detachArrayBuffer` goes through the engine's detach (`Cx::detach_array_buffer`):
    /// the buffer and its view get length 0; a second detach does nothing; a
    /// SharedArrayBuffer and a value that is not an engine buffer are TypeErrors. Plain and
    /// under GC stress 1, with a GC between the detach and the reads.
    #[test]
    fn detach_array_buffer_detaches_an_engine_buffer_and_its_views() {
        for gc_stress in [0, 1] {
            let mut iso = isolate(gc_stress);
            let r = iso.with_cx(|cx| {
                cx.evaluate_script(prelude(), "harness/$262 prelude")?;
                let buf = cx.allocate_array_buffer(Local::null(), 16, None)?;
                global(cx, "buf", buf.into())?;
                // Int32 (Table 71 row 5), bytes 4..12.
                let view = cx.new_typed_array(Local::null(), 5, buf, 4, Some(2))?;
                global(cx, "view", view.into())?;
                let sab = cx.allocate_shared_array_buffer(Local::null(), 8, None)?;
                global(cx, "sab", sab.into())?;
                let before = (
                    cx.array_buffer_byte_length(buf)?,
                    cx.typed_array_length(view)?,
                );
                cx.evaluate_script(
                    "$262.detachArrayBuffer(buf);\n\
                     $262.detachArrayBuffer(buf);\n\
                     var kinds = [sab, {}, 1, undefined].map(function (v) {\n\
                       try { $262.detachArrayBuffer(v); return 'none'; }\n\
                       catch (e) { return e instanceof TypeError ? 'TypeError' : String(e); }\n\
                     });\n\
                     if (kinds.join() !== 'TypeError,TypeError,TypeError,TypeError')\n\
                       throw new Error(kinds.join());",
                    "test",
                )?;
                cx.collect_garbage();
                let g = cx.global();
                let (buf, view, sab) = (cx.get(g, "buf")?, cx.get(g, "view")?, cx.get(g, "sab")?);
                Ok((
                    before,
                    cx.is_detached_buffer(buf),
                    cx.array_buffer_byte_length(buf)?,
                    cx.typed_array_length(view)?,
                    cx.typed_array_fixed_length(view)?,
                    cx.is_detached_buffer(sab),
                    cx.array_buffer_byte_length(sab)?,
                ))
            });
            assert_eq!(
                r.map_err(|e| e.to_string()),
                Ok(((16, Some(2)), true, 0, None, Some(0), false, 8)),
                "gc_stress {gc_stress}"
            );
        }
    }

    /// `$262.agent.broadcast` gives a started agent its own SharedArrayBuffer on the same block
    /// (H22-Q8; TA-2 fixer N1): the agent sees the bytes of the main agent, the main agent
    /// sees the agent's store (a DataView read and an Atomics load, `Cx::atomic_load`), a grow
    /// in the main agent shows the new byteLength in the agent, and the agent's object has the
    /// length, maximum and prototype of its realm. Two broadcasts of one SharedArrayBuffer give
    /// the agent two objects on one block: a species constructor of `slice` that returns the
    /// other one is a TypeError (step 18, the block test). After the agent's objects die and
    /// a GC runs there, the main agent still reads the block. A value that is not an engine
    /// SharedArrayBuffer (`{}`, an ArrayBuffer) is a TypeError. Plain and under GC stress 1.
    #[test]
    fn broadcast_shares_the_block_with_an_agent() {
        const AGENT: &str = r#"
$262.agent.receiveBroadcast(function (sab, n) {
  var d = new DataView(sab);
  var before = d.getInt32(4, true);
  d.setInt32(0, 42, true);
  $262.agent.report([n, before, sab.byteLength, sab.growable, sab.maxByteLength,
    Object.getPrototypeOf(sab) === SharedArrayBuffer.prototype].join());
  $262.agent.receiveBroadcast(function (sab2, m) {
    var r;
    sab.constructor = { [Symbol.species]: function () { return sab2; } };
    try { sab.slice(0, 4); r = 'no error'; } catch (e) { r = e.constructor.name; }
    $262.agent.report([m, sab.byteLength, sab2 === sab, r].join());
    sab = sab2 = d = null;
    $262.gc();
    $262.agent.leaving();
  });
});
"#;
        const MAIN: &str = r#"
function nextReport() {
  var r;
  while ((r = $262.agent.getReport()) == null) { $262.agent.sleep(1); }
  return r;
}
var sab = new SharedArrayBuffer(8, { maxByteLength: 64 });
var d = new DataView(sab);
d.setInt32(4, 7, true);
$262.agent.start(agentSource);
$262.agent.broadcast(sab, 1);
var first = nextReport();
var stored = d.getInt32(0, true);
sab.grow(32);
$262.agent.broadcast(sab, 2);
var second = nextReport();
$262.gc();
var errs = [{}, new ArrayBuffer(8)].map(function (v) {
  try { $262.agent.broadcast(v, 1); return 'none'; } catch (e) { return e.constructor.name; }
});
var out = [first, stored, second, d.getInt32(0, true), errs.join()].join(' | ');
"#;
        for gc_stress in [0, 1] {
            let mut iso = isolate(gc_stress);
            let config = AgentConfig {
                gc_stress,
                feedback_threshold: 0,
                exec: ExecOptions::default(),
            };
            // The session deadline ends every wait (no hang); it is not a time bound.
            let session = Session::begin(config, Duration::from_secs(120), Hooks::default());
            let r = iso.with_cx(|cx| {
                cx.evaluate_script(prelude(), "harness/$262 prelude")?;
                let src = cx.new_string(AGENT)?;
                global(cx, "agentSource", src.into())?;
                cx.evaluate_script(MAIN, "test")?;
                let g = cx.global();
                let out = cx.get(g, "out")?;
                let sab = cx.get(g, "sab")?;
                // Int32 (Table 71 row 5) at byte 0: the agent's store, seen by an Atomics load.
                let x = cx.atomic_load(sab, 0, 5)?;
                Ok((cx.to_rust_string(out)?, x.as_number()))
            });
            let end = session.end();
            assert_eq!(
                r.map_err(|e| e.to_string()),
                Ok((
                    "1,7,8,true,64,true | 42 | 2,32,false,TypeError | 42 | TypeError,TypeError"
                        .to_string(),
                    Some(42.0)
                )),
                "gc_stress {gc_stress}"
            );
            assert_eq!(
                (end.stuck, end.errors, end.panics, end.deadline_hit),
                (0, Vec::new(), Vec::new(), false),
                "gc_stress {gc_stress}"
            );
        }
    }

    /// The second value of a broadcast goes to the agent as a Number or a BigInt of the
    /// agent's realm, the same value (here: two isolates, as the main agent and an agent).
    #[test]
    fn a_broadcast_value_crosses_isolates() {
        let mut main = isolate(0);
        let sent = main.with_cx(|cx| {
            cx.evaluate_script("var big = -(2n ** 70n) + 1n;", "test")?;
            let g = cx.global();
            let big = cx.get(g, "big")?;
            let n = Local::number(-2.5);
            let u = Local::undefined();
            Ok((payload(cx, big)?, payload(cx, n)?, payload(cx, u)?))
        });
        let (big, n, u) = sent.expect("payloads");
        let mut agent = isolate(1);
        let got = agent.with_cx(|cx| {
            let mut out = Vec::new();
            for p in [&big, &n, &u] {
                let v = payload_value(cx, p)?;
                global(cx, "v", v)?;
                cx.evaluate_script("var s = typeof v + ' ' + String(v);", "test")?;
                let g = cx.global();
                let s = cx.get(g, "s")?;
                out.push(cx.to_rust_string(s)?);
            }
            Ok(out)
        });
        assert_eq!(
            got.map_err(|e| e.to_string()),
            Ok(vec![
                "bigint -1180591620717411303423".to_string(),
                "number -2.5".to_string(),
                "undefined undefined".to_string(),
            ])
        );
        let bad = agent.with_cx(|cx| {
            let o = cx.new_object();
            payload(cx, o.into()).map(|_| ())
        });
        assert_eq!(
            bad.map_err(|e| e.message),
            Err(
                "TypeError: $262.agent.broadcast: the second argument is not a Number or a \
                 BigInt"
                    .to_string()
            )
        );
    }

    /// Each prelude names only host functions of the list, and the list has no name twice.
    #[test]
    fn every_prelude_name_is_a_host_function() {
        let names: Vec<&str> = exec::HOST_FUNCTIONS.iter().map(|f| f.0).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "{names:?}");
        for text in [MAIN_TAIL, AGENT_TAIL, exec::PRELUDE] {
            for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                if word.starts_with("__ep9_") {
                    assert!(names.contains(&word), "{word} is not a host function");
                }
            }
        }
        assert!(prelude().starts_with(exec::PRELUDE) && prelude().ends_with(MAIN_TAIL));
    }

    /// The result of a run with its agents: an infrastructure error stays; an agent panic, a
    /// stuck agent and a wait past the deadline are infrastructure errors; an agent error
    /// fails a passing run and joins a failing run's detail.
    #[test]
    fn merge_rules() {
        let name = |r: &Run| match r {
            Run::Pass => "pass".to_string(),
            Run::Fail { cause, detail } => format!("fail {cause} | {detail}"),
            Run::Infra { cause, detail } => format!("infra {cause} | {detail}"),
        };
        let errors = End {
            errors: vec!["agent 0: TypeError: x".into()],
            ..End::default()
        };
        let clean = End::default();
        let cases: [(Run, End, &str); 7] = [
            (Run::Pass, End::default(), "pass"),
            (
                Run::Pass,
                End {
                    panics: vec!["agent 1: boom".into()],
                    ..End::default()
                },
                "infra panic | panic in an agent: agent 1: boom",
            ),
            (
                Run::Pass,
                End {
                    stuck: 2,
                    ..End::default()
                },
                "infra timeout | timeout: 2 agent(s) still ran 5s after the run ended and was \
                 terminated",
            ),
            (
                exec::fail("a", "b"),
                End {
                    deadline_hit: true,
                    ..End::default()
                },
                "infra timeout | timeout: a $262.agent wait passed the run's deadline",
            ),
            (
                Run::Pass,
                errors,
                "fail agent threw: agent 0: TypeError: x | agent errors: agent 0: TypeError: x",
            ),
            (
                exec::fail("c", "d"),
                End {
                    errors: vec!["agent 0: E".into()],
                    ..End::default()
                },
                "fail c | d; agent errors: agent 0: E",
            ),
            (
                exec::infra("timeout", "t"),
                End { stuck: 1, ..clean },
                "infra timeout | t",
            ),
        ];
        for (run, end, want) in cases {
            assert_eq!(name(&merge(run, &end)), want);
        }
    }
}
