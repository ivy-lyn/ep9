//! The facts exporter at the JS level (H16-Q5; V8-A12): richards (V8 benchmark suite v7,
//! `richards.js` at the `third_party/bench-v8v7.commit` pin, below with its BSD notice; the
//! `BenchmarkSuite` lines are left out) runs on an isolate with the default feedback
//! threshold; then `ep9_vm::abi::export_facts` takes the facts of its unit. The test checks
//! the facts of the hot functions, the round trip (export, encode, decode, equal), the
//! validator, and that a second isolate gives the same bytes.
//!
//! ep9_vm tests cannot compile JS, so this test lives here (ep9_facts is a dev-dependency
//! of ep9_builtins since M3-P0).

use ep9_bytecode::{UnitString, VerifiedUnit};
use ep9_facts::{
    Access, BindingSet, FactSnapshot, FunctionRef, IcState, KeyRef, ObjectKind, PropertyCase,
    ProtoRef, ShapeDesc, SlotFacts,
};
use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_heap::HeapConfig;
use ep9_vm::abi::export_facts;
use ep9_vm::{Isolate, IsolateConfig};

/// richards, then 3 runs (the first calls make the feedback vectors: 8 calls, or a loop's
/// budget poll).
fn source() -> String {
    format!("{RICHARDS}\nrunRichards();\nrunRichards();\nrunRichards();\n")
}

fn compile(src: &str) -> VerifiedUnit {
    ep9_frontend::compile(
        src,
        SourceOpts::script(Strictness::Sloppy),
        CompileOptions::default(),
    )
    .expect("richards compiles")
}

/// Runs richards on a fresh isolate: its unit and the facts of that unit.
fn run(src: &str) -> (VerifiedUnit, FactSnapshot) {
    run_with(src, IsolateConfig::default().feedback_threshold, false)
}

/// Runs the script `src` on a fresh isolate whose feedback threshold is `threshold`, with a
/// GC at each allocation if `stress`: its unit and the facts of that unit.
fn run_with(src: &str, threshold: u32, stress: bool) -> (VerifiedUnit, FactSnapshot) {
    let unit = compile(src);
    let mut iso = Isolate::new(IsolateConfig {
        builtins: Some(&ep9_builtins::TABLE),
        feedback_threshold: threshold,
        heap: HeapConfig {
            gc_stress: u32::from(stress),
            verify_after_gc: stress,
            ..HeapConfig::default()
        },
        ..IsolateConfig::default()
    })
    .expect("an isolate");
    let u = unit.clone();
    iso.with_cx(|cx| cx.run_script(u, "facts.js").map(|_| ()))
        .unwrap_or_else(|e| panic!("the script threw: {}", e.message));
    let facts = export_facts(&iso, &unit.content_hash()).expect("the unit is loaded");
    (unit, facts)
}

fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .expect("a thread")
        .join()
        .expect("no panic")
}

/// The id of the function whose source starts first at or after the end of `marker`.
fn function_after(unit: &VerifiedUnit, src: &str, marker: &str) -> u32 {
    let at = src
        .find(marker)
        .unwrap_or_else(|| panic!("{marker:?} not in richards"))
        + marker.len();
    let (id, _) = unit
        .functions
        .iter()
        .enumerate()
        .filter(|(_, f)| f.source_start as usize >= at)
        .min_by_key(|(_, f)| f.source_start)
        .expect("a function after the marker");
    id as u32
}

/// The records of function `f`.
fn records(s: &FactSnapshot, f: u32) -> Vec<&SlotFacts> {
    s.records()
        .filter(|(k, _)| k.function == f)
        .map(|(_, r)| r)
        .collect()
}

#[test]
fn richards_facts_round_trip() {
    on_big_stack(|| {
        let src = source();
        let (unit, facts) = run(&src);
        // A receiver shape whose first key is `first`.
        let starts = |s: &ShapeDesc, first: &str| {
            s.keys.first().map(|(k, _)| k) == Some(&KeyRef::String(UnitString::new(first)))
        };

        // Negative check first: richards makes its objects with `function` declarations, whose
        // `prototype` the VM keeps no proof for (Codex C-FX2), so no shape names a function
        // prototype; the prototypes are `unknown`.
        for (key, r) in facts.records() {
            r.for_each_shape(&mut |s| {
                assert!(
                    !matches!(s.proto, ProtoRef::FunctionPrototype(_)),
                    "{key:?}: {s:?}"
                );
            });
        }

        // Scheduler.prototype.schedule: `this.currentTcb.isHeldOrSuspended()` loads the method
        // from TaskControlBlock.prototype (a prototype field, depth 1, on a TaskControlBlock:
        // its first key is `link`), and `this.currentTcb` is an own field of a Scheduler (its
        // first key is `queueCount`).
        let schedule = function_after(&unit, &src, "Scheduler.prototype.schedule = ");
        let rs = records(&facts, schedule);
        assert!(!rs.is_empty(), "schedule has no records");
        let cases: Vec<_> = rs
            .iter()
            .filter_map(|r| match r {
                SlotFacts::LoadNamed(p) => Some(p),
                _ => None,
            })
            .flat_map(|p| p.cases.iter())
            .collect();
        assert!(
            cases
                .iter()
                .any(|c| matches!(c.access, Access::ProtoField { depth: 1, .. })
                    && c.receiver.proto == ProtoRef::Unknown
                    && starts(&c.receiver, "link")
                    && c.receiver.kind == ObjectKind::Ordinary),
            "no TaskControlBlock method load in schedule: {cases:?}"
        );
        assert!(
            cases
                .iter()
                .any(|c| matches!(c.access, Access::OwnField { .. })
                    && c.receiver.proto == ProtoRef::Unknown
                    && starts(&c.receiver, "queueCount")),
            "no Scheduler field load in schedule: {cases:?}"
        );

        // TaskControlBlock.prototype.isHeldOrSuspended: `this.state` (own field) and the
        // globals STATE_HELD, STATE_SUSPENDED (data properties of the global object).
        let held = function_after(
            &unit,
            &src,
            "TaskControlBlock.prototype.isHeldOrSuspended = ",
        );
        let rs = records(&facts, held);
        assert!(rs.iter().any(|r| matches!(r, SlotFacts::LoadNamed(p)
            if p.state == IcState::Monomorphic
                && matches!(p.cases[..], [ref c] if matches!(c.access, Access::OwnField { .. })))));
        let globals: Vec<_> = rs
            .iter()
            .filter_map(|r| match r {
                SlotFacts::LoadGlobal(g) => Some(g.bindings),
                _ => None,
            })
            .collect();
        assert_eq!(globals, [BindingSet::DATA, BindingSet::DATA], "{rs:?}");

        // The Packet constructor: `this.link = link` adds a field (an add case).
        let packet = function_named(&unit, "Packet");
        assert!(
            records(&facts, packet)
                .iter()
                .any(|r| matches!(r, SlotFacts::StoreNamed(p)
                if p.cases.iter().any(|c| matches!(c.access, Access::AddField { .. })))),
            "no add case in Packet: {:?}",
            records(&facts, packet)
        );

        // Round trip: export, encode, decode, equal; the bytes are canonical.
        let bytes = facts.encode();
        let back = FactSnapshot::decode(&bytes).expect("the exporter's bytes decode");
        assert_eq!(back, facts);
        assert_eq!(back.encode(), bytes);
        assert_eq!(back.dump(), facts.dump());
        // The validator keeps every record for this unit.
        let (valid, dropped) = back.validate(&unit).expect("not stale");
        assert!(dropped.is_empty(), "{dropped:?}");
        assert_eq!(valid, facts);

        // A second isolate gives the same bytes: no ShapeId, address or closure identity.
        let (_, again) = run(&src);
        assert_eq!(again.encode(), bytes);
    });
}

/// The id of the function named `name` (a declaration, a class or a `var` initializer).
fn function_named(unit: &VerifiedUnit, name: &str) -> u32 {
    unit.functions
        .iter()
        .position(|f| {
            f.name
                .is_some_and(|n| unit.strings[n.index()] == UnitString::new(name))
        })
        .unwrap_or_else(|| panic!("no function {name}")) as u32
}

/// The cases of the one property record of function `f`.
fn property_cases(s: &FactSnapshot, f: u32) -> Vec<PropertyCase> {
    let rs = records(s, f);
    let props: Vec<_> = rs
        .iter()
        .filter_map(|r| match r {
            SlotFacts::LoadNamed(p) | SlotFacts::StoreNamed(p) => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(props.len(), 1, "{rs:?}");
    props[0].cases.clone()
}

/// The bindings of the one global record of function `f`.
fn global_bindings(s: &FactSnapshot, f: u32) -> BindingSet {
    let rs = records(s, f);
    let globals: Vec<_> = rs
        .iter()
        .filter_map(|r| match r {
            SlotFacts::LoadGlobal(g) | SlotFacts::StoreGlobal(g) => Some(g.bindings),
            _ => None,
        })
        .collect();
    assert_eq!(globals.len(), 1, "{rs:?}");
    globals[0]
}

/// Export, encode, decode, equal; the validator keeps every record.
fn assert_round_trip(unit: &VerifiedUnit, facts: &FactSnapshot) {
    let bytes = facts.encode();
    let back = FactSnapshot::decode(&bytes).expect("the exporter's bytes decode");
    assert_eq!(&back, facts);
    assert_eq!(back.encode(), bytes);
    let (valid, dropped) = back.validate(unit).expect("not stale");
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(&valid, facts);
}

/// Codex C-FX1 (M3-P3 round 1): the global object can have a null prototype (it becomes a
/// prototype first, so its shape stays fast). A `SLOW` global case keyed on its shape is then
/// not a lexical binding: an accessor is `accessor-property`, a read-only data property is
/// `data-property`. The JS lines give the node v24.22.1 results.
#[test]
fn a_null_prototype_global_is_not_lexical() {
    on_big_stack(|| {
        let src = r#"
var heir = Object.create(globalThis);
Object.setPrototypeOf(globalThis, null);
Object.defineProperty(globalThis, "q", { get: function () { return 7; }, configurable: true });
Object.defineProperty(globalThis, "ro", { value: 1, writable: false, configurable: true });
const k = 1;
let lv = 2;
function readQ() { return q; }
function writeRo() { ro = 2; }
function writeK() { try { k = 3; } catch (e) { return e.constructor.name; } }
function readLv() { return lv; }
for (var i = 0; i < 20; i++) {
  var got = [readQ(), writeRo(), ro, writeK(), readLv()].join();
  if (got !== "7,,1,TypeError,2") throw new Error(got);
}
if (Object.getPrototypeOf(globalThis) !== null) throw new Error("prototype");
"#;
        for stress in [false, true] {
            let (unit, facts) = run_with(src, 0, stress);
            let f = |name| global_bindings(&facts, function_named(&unit, name));
            // Negative checks first: no global-object case is lexical.
            assert!(
                !f("readQ").contains(BindingSet::LEXICAL),
                "{:?}",
                f("readQ")
            );
            assert!(
                !f("writeRo").contains(BindingSet::LEXICAL),
                "{:?}",
                f("writeRo")
            );
            assert_eq!(f("readQ"), BindingSet::ACCESSOR);
            assert_eq!(f("writeRo"), BindingSet::DATA);
            // A `let` load stays lexical (a field case). A store to a `const` throws, and the miss
            // path records no case for a store that throws: no binding.
            assert_eq!(f("readLv"), BindingSet::LEXICAL);
            assert_eq!(f("writeK"), BindingSet::EMPTY);
            assert_round_trip(&unit, &facts);
        }
    });
}

/// Codex C-FX2 (M3-P3 round 1): `constructor` and `prototype` are writable, so a backlink
/// pair does not prove that an object is the initial prototype of a function. Only a class
/// keeps that proof (its `prototype` is {w: false, c: false} from DefineClass, and its
/// `[[HomeObject]]` is that object), so only a class instance's prototype is
/// `function-prototype`; the replaced, forged and arrow cases, and a `function` declaration's
/// prototype (no proof kept), are `unknown`. The JS lines give the node v24.22.1 results.
#[test]
fn only_a_proven_initial_prototype_is_named() {
    on_big_stack(|| {
        let src = r#"
function C() {}
C.prototype = { constructor: C };
var A = () => {};
A.prototype = { constructor: A };
function D() {}
var dp = Object.create(Object.prototype);
Object.defineProperty(dp, "constructor", { value: D, writable: true, configurable: true });
D.prototype = dp;
function E() {}
class K { constructor() { this.x = 1; } }
class J {}
J.prototype.constructor = K;
function readC(o) { return o.x; }
function readA(o) { return o.x; }
function readD(o) { return o.x; }
function readE(o) { return o.x; }
function readK(o) { return o.x; }
function readJ(o) { return o.x; }
var c = new C(), a = Object.create(A.prototype), d = new D(), e = new E(), k = new K(),
  j = new J();
for (var i = 0; i < 20; i++) {
  var got = [readC(c), readA(a), readD(d), readE(e), readK(k), readJ(j)].join();
  if (got !== ",,,,1,") throw new Error(got);
}
if (Object.getPrototypeOf(a).constructor !== A || d.constructor !== D) throw new Error("links");
if (j.constructor !== K || Object.getOwnPropertyDescriptor(K, "prototype").writable)
  throw new Error("class");
"#;
        for stress in [false, true] {
            let (unit, facts) = run_with(src, 0, stress);
            let hash = unit.content_hash();
            let proto = |reader: &str| {
                let cases = property_cases(&facts, function_named(&unit, reader));
                assert_eq!(cases.len(), 1, "{reader}: {cases:?}");
                cases[0].receiver.proto
            };
            let named = |f: &str| {
                ProtoRef::FunctionPrototype(FunctionRef::Js {
                    unit: hash,
                    function: function_named(&unit, f),
                })
            };
            // Negative checks first: no forged or unproven backlink names a function.
            for (reader, f) in [
                ("readC", "C"),
                ("readA", "A"),
                ("readD", "D"),
                ("readE", "E"),
                ("readJ", "K"),
                ("readJ", "J"),
            ] {
                assert_ne!(proto(reader), named(f), "{reader}");
                assert_eq!(proto(reader), ProtoRef::Unknown, "{reader}");
            }
            assert_eq!(proto("readK"), named("K"));
            assert_round_trip(&unit, &facts);
        }
    });
}

const RICHARDS: &str = r##"// Copyright 2006-2008 the V8 project authors. All rights reserved.
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above
//       copyright notice, this list of conditions and the following
//       disclaimer in the documentation and/or other materials provided
//       with the distribution.
//     * Neither the name of Google Inc. nor the names of its
//       contributors may be used to endorse or promote products derived
//       from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
// "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
// LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
// A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
// OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
// LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
// DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
// THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.


// This is a JavaScript implementation of the Richards
// benchmark from:
//
//    http://www.cl.cam.ac.uk/~mr10/Bench.html
//
// The benchmark was originally implemented in BCPL by
// Martin Richards.


/**
 * The Richards benchmark simulates the task dispatcher of an
 * operating system.
 **/
function runRichards() {
  var scheduler = new Scheduler();
  scheduler.addIdleTask(ID_IDLE, 0, null, COUNT);

  var queue = new Packet(null, ID_WORKER, KIND_WORK);
  queue = new Packet(queue,  ID_WORKER, KIND_WORK);
  scheduler.addWorkerTask(ID_WORKER, 1000, queue);

  queue = new Packet(null, ID_DEVICE_A, KIND_DEVICE);
  queue = new Packet(queue,  ID_DEVICE_A, KIND_DEVICE);
  queue = new Packet(queue,  ID_DEVICE_A, KIND_DEVICE);
  scheduler.addHandlerTask(ID_HANDLER_A, 2000, queue);

  queue = new Packet(null, ID_DEVICE_B, KIND_DEVICE);
  queue = new Packet(queue,  ID_DEVICE_B, KIND_DEVICE);
  queue = new Packet(queue,  ID_DEVICE_B, KIND_DEVICE);
  scheduler.addHandlerTask(ID_HANDLER_B, 3000, queue);

  scheduler.addDeviceTask(ID_DEVICE_A, 4000, null);

  scheduler.addDeviceTask(ID_DEVICE_B, 5000, null);

  scheduler.schedule();

  if (scheduler.queueCount != EXPECTED_QUEUE_COUNT ||
      scheduler.holdCount != EXPECTED_HOLD_COUNT) {
    var msg =
        "Error during execution: queueCount = " + scheduler.queueCount +
        ", holdCount = " + scheduler.holdCount + ".";
    throw new Error(msg);
  }
}

var COUNT = 1000;

/**
 * These two constants specify how many times a packet is queued and
 * how many times a task is put on hold in a correct run of richards.
 * They don't have any meaning a such but are characteristic of a
 * correct run so if the actual queue or hold count is different from
 * the expected there must be a bug in the implementation.
 **/
var EXPECTED_QUEUE_COUNT = 2322;
var EXPECTED_HOLD_COUNT = 928;


/**
 * A scheduler can be used to schedule a set of tasks based on their relative
 * priorities.  Scheduling is done by maintaining a list of task control blocks
 * which holds tasks and the data queue they are processing.
 * @constructor
 */
function Scheduler() {
  this.queueCount = 0;
  this.holdCount = 0;
  this.blocks = new Array(NUMBER_OF_IDS);
  this.list = null;
  this.currentTcb = null;
  this.currentId = null;
}

var ID_IDLE       = 0;
var ID_WORKER     = 1;
var ID_HANDLER_A  = 2;
var ID_HANDLER_B  = 3;
var ID_DEVICE_A   = 4;
var ID_DEVICE_B   = 5;
var NUMBER_OF_IDS = 6;

var KIND_DEVICE   = 0;
var KIND_WORK     = 1;

/**
 * Add an idle task to this scheduler.
 * @param {int} id the identity of the task
 * @param {int} priority the task's priority
 * @param {Packet} queue the queue of work to be processed by the task
 * @param {int} count the number of times to schedule the task
 */
Scheduler.prototype.addIdleTask = function (id, priority, queue, count) {
  this.addRunningTask(id, priority, queue, new IdleTask(this, 1, count));
};

/**
 * Add a work task to this scheduler.
 * @param {int} id the identity of the task
 * @param {int} priority the task's priority
 * @param {Packet} queue the queue of work to be processed by the task
 */
Scheduler.prototype.addWorkerTask = function (id, priority, queue) {
  this.addTask(id, priority, queue, new WorkerTask(this, ID_HANDLER_A, 0));
};

/**
 * Add a handler task to this scheduler.
 * @param {int} id the identity of the task
 * @param {int} priority the task's priority
 * @param {Packet} queue the queue of work to be processed by the task
 */
Scheduler.prototype.addHandlerTask = function (id, priority, queue) {
  this.addTask(id, priority, queue, new HandlerTask(this));
};

/**
 * Add a handler task to this scheduler.
 * @param {int} id the identity of the task
 * @param {int} priority the task's priority
 * @param {Packet} queue the queue of work to be processed by the task
 */
Scheduler.prototype.addDeviceTask = function (id, priority, queue) {
  this.addTask(id, priority, queue, new DeviceTask(this));
};

/**
 * Add the specified task and mark it as running.
 * @param {int} id the identity of the task
 * @param {int} priority the task's priority
 * @param {Packet} queue the queue of work to be processed by the task
 * @param {Task} task the task to add
 */
Scheduler.prototype.addRunningTask = function (id, priority, queue, task) {
  this.addTask(id, priority, queue, task);
  this.currentTcb.setRunning();
};

/**
 * Add the specified task to this scheduler.
 * @param {int} id the identity of the task
 * @param {int} priority the task's priority
 * @param {Packet} queue the queue of work to be processed by the task
 * @param {Task} task the task to add
 */
Scheduler.prototype.addTask = function (id, priority, queue, task) {
  this.currentTcb = new TaskControlBlock(this.list, id, priority, queue, task);
  this.list = this.currentTcb;
  this.blocks[id] = this.currentTcb;
};

/**
 * Execute the tasks managed by this scheduler.
 */
Scheduler.prototype.schedule = function () {
  this.currentTcb = this.list;
  while (this.currentTcb != null) {
    if (this.currentTcb.isHeldOrSuspended()) {
      this.currentTcb = this.currentTcb.link;
    } else {
      this.currentId = this.currentTcb.id;
      this.currentTcb = this.currentTcb.run();
    }
  }
};

/**
 * Release a task that is currently blocked and return the next block to run.
 * @param {int} id the id of the task to suspend
 */
Scheduler.prototype.release = function (id) {
  var tcb = this.blocks[id];
  if (tcb == null) return tcb;
  tcb.markAsNotHeld();
  if (tcb.priority > this.currentTcb.priority) {
    return tcb;
  } else {
    return this.currentTcb;
  }
};

/**
 * Block the currently executing task and return the next task control block
 * to run.  The blocked task will not be made runnable until it is explicitly
 * released, even if new work is added to it.
 */
Scheduler.prototype.holdCurrent = function () {
  this.holdCount++;
  this.currentTcb.markAsHeld();
  return this.currentTcb.link;
};

/**
 * Suspend the currently executing task and return the next task control block
 * to run.  If new work is added to the suspended task it will be made runnable.
 */
Scheduler.prototype.suspendCurrent = function () {
  this.currentTcb.markAsSuspended();
  return this.currentTcb;
};

/**
 * Add the specified packet to the end of the worklist used by the task
 * associated with the packet and make the task runnable if it is currently
 * suspended.
 * @param {Packet} packet the packet to add
 */
Scheduler.prototype.queue = function (packet) {
  var t = this.blocks[packet.id];
  if (t == null) return t;
  this.queueCount++;
  packet.link = null;
  packet.id = this.currentId;
  return t.checkPriorityAdd(this.currentTcb, packet);
};

/**
 * A task control block manages a task and the queue of work packages associated
 * with it.
 * @param {TaskControlBlock} link the preceding block in the linked block list
 * @param {int} id the id of this block
 * @param {int} priority the priority of this block
 * @param {Packet} queue the queue of packages to be processed by the task
 * @param {Task} task the task
 * @constructor
 */
function TaskControlBlock(link, id, priority, queue, task) {
  this.link = link;
  this.id = id;
  this.priority = priority;
  this.queue = queue;
  this.task = task;
  if (queue == null) {
    this.state = STATE_SUSPENDED;
  } else {
    this.state = STATE_SUSPENDED_RUNNABLE;
  }
}

/**
 * The task is running and is currently scheduled.
 */
var STATE_RUNNING = 0;

/**
 * The task has packets left to process.
 */
var STATE_RUNNABLE = 1;

/**
 * The task is not currently running.  The task is not blocked as such and may
* be started by the scheduler.
 */
var STATE_SUSPENDED = 2;

/**
 * The task is blocked and cannot be run until it is explicitly released.
 */
var STATE_HELD = 4;

var STATE_SUSPENDED_RUNNABLE = STATE_SUSPENDED | STATE_RUNNABLE;
var STATE_NOT_HELD = ~STATE_HELD;

TaskControlBlock.prototype.setRunning = function () {
  this.state = STATE_RUNNING;
};

TaskControlBlock.prototype.markAsNotHeld = function () {
  this.state = this.state & STATE_NOT_HELD;
};

TaskControlBlock.prototype.markAsHeld = function () {
  this.state = this.state | STATE_HELD;
};

TaskControlBlock.prototype.isHeldOrSuspended = function () {
  return (this.state & STATE_HELD) != 0 || (this.state == STATE_SUSPENDED);
};

TaskControlBlock.prototype.markAsSuspended = function () {
  this.state = this.state | STATE_SUSPENDED;
};

TaskControlBlock.prototype.markAsRunnable = function () {
  this.state = this.state | STATE_RUNNABLE;
};

/**
 * Runs this task, if it is ready to be run, and returns the next task to run.
 */
TaskControlBlock.prototype.run = function () {
  var packet;
  if (this.state == STATE_SUSPENDED_RUNNABLE) {
    packet = this.queue;
    this.queue = packet.link;
    if (this.queue == null) {
      this.state = STATE_RUNNING;
    } else {
      this.state = STATE_RUNNABLE;
    }
  } else {
    packet = null;
  }
  return this.task.run(packet);
};

/**
 * Adds a packet to the worklist of this block's task, marks this as runnable if
 * necessary, and returns the next runnable object to run (the one
 * with the highest priority).
 */
TaskControlBlock.prototype.checkPriorityAdd = function (task, packet) {
  if (this.queue == null) {
    this.queue = packet;
    this.markAsRunnable();
    if (this.priority > task.priority) return this;
  } else {
    this.queue = packet.addTo(this.queue);
  }
  return task;
};

TaskControlBlock.prototype.toString = function () {
  return "tcb { " + this.task + "@" + this.state + " }";
};

/**
 * An idle task doesn't do any work itself but cycles control between the two
 * device tasks.
 * @param {Scheduler} scheduler the scheduler that manages this task
 * @param {int} v1 a seed value that controls how the device tasks are scheduled
 * @param {int} count the number of times this task should be scheduled
 * @constructor
 */
function IdleTask(scheduler, v1, count) {
  this.scheduler = scheduler;
  this.v1 = v1;
  this.count = count;
}

IdleTask.prototype.run = function (packet) {
  this.count--;
  if (this.count == 0) return this.scheduler.holdCurrent();
  if ((this.v1 & 1) == 0) {
    this.v1 = this.v1 >> 1;
    return this.scheduler.release(ID_DEVICE_A);
  } else {
    this.v1 = (this.v1 >> 1) ^ 0xD008;
    return this.scheduler.release(ID_DEVICE_B);
  }
};

IdleTask.prototype.toString = function () {
  return "IdleTask";
};

/**
 * A task that suspends itself after each time it has been run to simulate
 * waiting for data from an external device.
 * @param {Scheduler} scheduler the scheduler that manages this task
 * @constructor
 */
function DeviceTask(scheduler) {
  this.scheduler = scheduler;
  this.v1 = null;
}

DeviceTask.prototype.run = function (packet) {
  if (packet == null) {
    if (this.v1 == null) return this.scheduler.suspendCurrent();
    var v = this.v1;
    this.v1 = null;
    return this.scheduler.queue(v);
  } else {
    this.v1 = packet;
    return this.scheduler.holdCurrent();
  }
};

DeviceTask.prototype.toString = function () {
  return "DeviceTask";
};

/**
 * A task that manipulates work packets.
 * @param {Scheduler} scheduler the scheduler that manages this task
 * @param {int} v1 a seed used to specify how work packets are manipulated
 * @param {int} v2 another seed used to specify how work packets are manipulated
 * @constructor
 */
function WorkerTask(scheduler, v1, v2) {
  this.scheduler = scheduler;
  this.v1 = v1;
  this.v2 = v2;
}

WorkerTask.prototype.run = function (packet) {
  if (packet == null) {
    return this.scheduler.suspendCurrent();
  } else {
    if (this.v1 == ID_HANDLER_A) {
      this.v1 = ID_HANDLER_B;
    } else {
      this.v1 = ID_HANDLER_A;
    }
    packet.id = this.v1;
    packet.a1 = 0;
    for (var i = 0; i < DATA_SIZE; i++) {
      this.v2++;
      if (this.v2 > 26) this.v2 = 1;
      packet.a2[i] = this.v2;
    }
    return this.scheduler.queue(packet);
  }
};

WorkerTask.prototype.toString = function () {
  return "WorkerTask";
};

/**
 * A task that manipulates work packets and then suspends itself.
 * @param {Scheduler} scheduler the scheduler that manages this task
 * @constructor
 */
function HandlerTask(scheduler) {
  this.scheduler = scheduler;
  this.v1 = null;
  this.v2 = null;
}

HandlerTask.prototype.run = function (packet) {
  if (packet != null) {
    if (packet.kind == KIND_WORK) {
      this.v1 = packet.addTo(this.v1);
    } else {
      this.v2 = packet.addTo(this.v2);
    }
  }
  if (this.v1 != null) {
    var count = this.v1.a1;
    var v;
    if (count < DATA_SIZE) {
      if (this.v2 != null) {
        v = this.v2;
        this.v2 = this.v2.link;
        v.a1 = this.v1.a2[count];
        this.v1.a1 = count + 1;
        return this.scheduler.queue(v);
      }
    } else {
      v = this.v1;
      this.v1 = this.v1.link;
      return this.scheduler.queue(v);
    }
  }
  return this.scheduler.suspendCurrent();
};

HandlerTask.prototype.toString = function () {
  return "HandlerTask";
};

/* --- *
 * P a c k e t
 * --- */

var DATA_SIZE = 4;

/**
 * A simple package of data that is manipulated by the tasks.  The exact layout
 * of the payload data carried by a packet is not importaint, and neither is the
 * nature of the work performed on packets by the tasks.
 *
 * Besides carrying data, packets form linked lists and are hence used both as
 * data and worklists.
 * @param {Packet} link the tail of the linked list of packets
 * @param {int} id an ID for this packet
 * @param {int} kind the type of this packet
 * @constructor
 */
function Packet(link, id, kind) {
  this.link = link;
  this.id = id;
  this.kind = kind;
  this.a1 = 0;
  this.a2 = new Array(DATA_SIZE);
}

/**
 * Add this packet to the end of a worklist, and return the worklist.
 * @param {Packet} queue the worklist to add this packet to
 */
Packet.prototype.addTo = function (queue) {
  this.link = null;
  if (queue == null) return this;
  var peek, next = queue;
  while ((peek = next.link) != null)
    next = peek;
  next.link = this;
  return queue;
};

Packet.prototype.toString = function () {
  return "Packet";
};
"##;
