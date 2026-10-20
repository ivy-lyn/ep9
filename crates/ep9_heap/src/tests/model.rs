//! The H11 barrier decision model (03-gc §7 "Barrier model test"), ported from the decision's
//! Python models (gc-barrier `model.py` and the critic's `los_card_model.py`) into one
//! transition system.
//!
//! One mutator and one concurrent marker; every interleaving of their sub-steps is explored
//! (sequential consistency between sub-steps, which is what the SeqCst fences buy). At the end
//! of each run the stop-the-world termination runs (if the form has it), and any object that
//! is reachable from the roots or registers but not marked is LOST.
//!
//! ```text
//! marker: pick a GREY object -> BLACK, clear its cards, snapshot its field names
//!         read one field per step; an unmarked cell -> marked + GREY
//! mutator: load | weak load | drop | new (WHITE) | store
//!   decided store: is_cell(v) && host not fresh -> store, then (next step) re-check:
//!                  LOS host -> dirty its card, push once if BLACK; else BLACK -> GREY
//! termination: rescan roots + registers + dirty cards of marked LOS hosts; drain
//! ```
//!
//! | Scenario | What it stresses |
//! |---|---|
//! | move | the only reference moves from A.f to B.g through a register |
//! | alloc | a new object N gets C, then N is published into B |
//! | reg | C survives only in a register |
//! | escape | N escapes into B first; a later store into N must not be elided (E2) |
//! | weak | W is reachable only through a weak cell; deref + store into B |
//! | los | C is stored into a large array L (cards) and deleted from A |
//!
//! Negative controls (03-gc §7) must lose an object: drop the barrier (`rw_nobarrier`),
//! old-only filter, no root rescan, elide after escape, no card rescan at termination
//! (`los_no_card_rescan`, the critic's "card without rescan or push" form).

use std::collections::{BTreeMap, BTreeSet, HashSet};

type Obj = &'static str;
type Field = &'static str;
type Reg = &'static str;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Val {
    Int,
    Cell(Obj),
}

impl Val {
    fn cell(self) -> Option<Obj> {
        match self {
            Val::Cell(o) => Some(o),
            Val::Int => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Src {
    Reg(Reg),
    Int,
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Load(Reg, Obj, Field),
    WeakLoad(Reg, &'static str),
    Drop(Reg),
    New(Reg, Obj),
    Store(Obj, Field, Src),
}

struct Scenario {
    name: &'static str,
    heap: &'static [(Obj, &'static [(Field, Val)])],
    roots: &'static [Obj],
    weak: &'static [(&'static str, Obj)],
    los: &'static [Obj],
    prog: &'static [Op],
}

const I: Val = Val::Int;
const fn c(o: Obj) -> Val {
    Val::Cell(o)
}

const SCENARIOS: [Scenario; 6] = [
    Scenario {
        name: "move",
        heap: &[
            ("R", &[("a", c("A")), ("b", c("B"))]),
            ("A", &[("f", c("C"))]),
            ("B", &[("g", I)]),
            ("C", &[]),
        ],
        roots: &["R"],
        weak: &[],
        los: &[],
        prog: &[
            Op::Load("x", "A", "f"),
            Op::Store("A", "f", Src::Int),
            Op::Store("B", "g", Src::Reg("x")),
            Op::Drop("x"),
        ],
    },
    Scenario {
        name: "alloc",
        heap: &[
            ("R", &[("a", c("A")), ("b", c("B"))]),
            ("A", &[("f", c("C"))]),
            ("B", &[("g", I)]),
            ("C", &[]),
        ],
        roots: &["R"],
        weak: &[],
        los: &[],
        prog: &[
            Op::Load("x", "A", "f"),
            Op::New("y", "N"),
            Op::Store("N", "h", Src::Reg("x")),
            Op::Store("A", "f", Src::Int),
            Op::Store("B", "g", Src::Reg("y")),
            Op::Drop("x"),
            Op::Drop("y"),
        ],
    },
    Scenario {
        name: "reg",
        heap: &[("R", &[("a", c("A"))]), ("A", &[("f", c("C"))]), ("C", &[])],
        roots: &["R"],
        weak: &[],
        los: &[],
        prog: &[Op::Load("x", "A", "f"), Op::Store("A", "f", Src::Int)],
    },
    Scenario {
        name: "escape",
        heap: &[
            ("R", &[("a", c("A")), ("b", c("B"))]),
            ("A", &[("f", c("C"))]),
            ("B", &[("g", I)]),
            ("C", &[]),
        ],
        roots: &["R"],
        weak: &[],
        los: &[],
        prog: &[
            Op::New("y", "N"),
            Op::Store("B", "g", Src::Reg("y")),
            Op::Drop("y"),
            Op::Load("x", "A", "f"),
            Op::Store("A", "f", Src::Int),
            Op::Load("y", "B", "g"),
            Op::Store("N", "h", Src::Reg("x")),
            Op::Drop("x"),
            Op::Drop("y"),
        ],
    },
    Scenario {
        name: "weak",
        heap: &[("R", &[("b", c("B"))]), ("B", &[("g", I)]), ("W", &[])],
        roots: &["R"],
        weak: &[("wk", "W")],
        los: &[],
        prog: &[
            Op::WeakLoad("x", "wk"),
            Op::Store("B", "g", Src::Reg("x")),
            Op::Drop("x"),
        ],
    },
    Scenario {
        name: "los",
        heap: &[
            ("R", &[("a", c("A")), ("l", c("L"))]),
            ("A", &[("f", c("C"))]),
            ("L", &[("e0", I), ("e1", I)]),
            ("C", &[]),
        ],
        roots: &["R"],
        weak: &[],
        los: &["L"],
        prog: &[
            Op::Load("x", "A", "f"),
            Op::Store("L", "e1", Src::Reg("x")),
            Op::Store("A", "f", Src::Int),
            Op::Drop("x"),
        ],
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Satb {
    /// Yuasa SATB, filter on the new value only (the first H11 draft).
    New,
    /// Yuasa SATB, filter old OR new (the M0 prototype).
    OldNew,
}

#[derive(Clone, Copy, Debug)]
struct Form {
    name: &'static str,
    satb: Option<Satb>,
    weak_read_barrier: bool,
    old_filter: bool,
    precheck: bool,
    rescan: bool,
    elide_escaped: bool,
    no_barrier: bool,
    los_push: bool,
    los_card_rescan: bool,
}

const DECIDED: Form = Form {
    name: "decided",
    satb: None,
    weak_read_barrier: false,
    old_filter: false,
    precheck: false,
    rescan: true,
    elide_escaped: false,
    no_barrier: false,
    los_push: true,
    los_card_rescan: true,
};

const fn satb(name: &'static str, s: Satb, weak_read_barrier: bool) -> Form {
    Form {
        name,
        satb: Some(s),
        weak_read_barrier,
        rescan: false,
        ..DECIDED
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Pending {
    None,
    Recheck(Obj),
    StoreAfter(Obj, Field, Val),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct State {
    heap: BTreeMap<Obj, BTreeMap<Field, Val>>,
    regs: BTreeMap<Reg, Val>,
    pc: usize,
    marked: BTreeSet<Obj>,
    black: BTreeSet<Obj>,
    grey: BTreeSet<Obj>,
    visiting: Option<(Obj, Vec<Field>)>,
    pending: Pending,
    new: BTreeSet<Obj>,
    escaped: BTreeSet<Obj>,
    dirty: BTreeSet<Obj>,
}

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    states: usize,
    bad_end_states: usize,
    lost: BTreeSet<Obj>,
}

fn is_cell(v: Option<Val>) -> bool {
    matches!(v, Some(Val::Cell(_)))
}

/// The slow path's log step (S2): LOS host -> card (+ push once if BLACK); else re-grey.
fn log_host(f: &Form, sc: &Scenario, s: &mut State, o: Obj) {
    if sc.los.contains(&o) {
        s.dirty.insert(o);
        if f.los_push && s.black.remove(o) {
            s.grey.insert(o);
        }
    } else if s.black.remove(o) {
        s.grey.insert(o);
    }
}

fn successors(f: &Form, sc: &Scenario, s: &State, out: &mut Vec<State>) {
    // ---- marker ----
    if let Some((o, left)) = &s.visiting {
        let mut n = s.clone();
        if let Some((&field, rest)) = left.split_first() {
            if let Some(Val::Cell(v)) = s.heap[o].get(field).copied()
                && n.marked.insert(v)
            {
                n.grey.insert(v);
            }
            n.visiting = Some((o, rest.to_vec()));
        } else {
            n.visiting = None;
        }
        out.push(n);
    } else {
        for &o in &s.grey {
            let mut n = s.clone();
            n.grey.remove(o);
            n.black.insert(o);
            n.dirty.remove(o); // LOS: clear the cards, fence, then scan
            n.visiting = Some((o, s.heap[o].keys().copied().collect()));
            out.push(n);
        }
    }
    // ---- mutator ----
    match &s.pending {
        Pending::StoreAfter(o, field, v) => {
            let mut n = s.clone();
            n.heap.get_mut(o).unwrap().insert(field, *v);
            n.pc += 1;
            n.pending = Pending::None;
            out.push(n);
            return;
        }
        Pending::Recheck(o) => {
            let mut n = s.clone();
            log_host(f, sc, &mut n, o);
            n.pending = Pending::None;
            out.push(n);
            return;
        }
        Pending::None => {}
    }
    let Some(op) = sc.prog.get(s.pc) else {
        return;
    };
    let mut n = s.clone();
    n.pc += 1;
    match *op {
        Op::Load(r, o, field) => {
            n.regs.insert(r, s.heap[o][field]);
        }
        Op::WeakLoad(r, wk) => {
            let w = sc.weak.iter().find(|(k, _)| *k == wk).unwrap().1;
            n.regs.insert(r, Val::Cell(w));
            if f.satb.is_some() && f.weak_read_barrier && n.marked.insert(w) {
                n.grey.insert(w);
            }
        }
        Op::Drop(r) => {
            n.regs.remove(r);
        }
        Op::New(r, name) => {
            n.heap.insert(name, BTreeMap::new());
            n.regs.insert(r, Val::Cell(name));
            n.new.insert(name);
            if f.satb.is_some() {
                // SATB allocates black.
                n.marked.insert(name);
                n.black.insert(name);
            }
        }
        Op::Store(o, field, src) => {
            let v = match src {
                Src::Reg(r) => s.regs[r],
                Src::Int => Val::Int,
            };
            let old = s.heap[o].get(field).copied();
            if let Val::Cell(vo) = v
                && n.new.contains(vo)
            {
                n.escaped.insert(vo);
            }
            let fresh = n.new.contains(o) && (f.elide_escaped || !n.escaped.contains(o));
            if let Some(kind) = f.satb {
                let fire = match kind {
                    Satb::New => is_cell(Some(v)),
                    Satb::OldNew => is_cell(old) || is_cell(Some(v)),
                };
                if fire
                    && !n.new.contains(o)
                    && let Some(Val::Cell(oc)) = old
                    && n.marked.insert(oc)
                {
                    n.grey.insert(oc); // Yuasa: shade the old value
                }
                n.heap.get_mut(o).unwrap().insert(field, v);
            } else {
                let mut fire = if f.old_filter {
                    is_cell(old)
                } else {
                    is_cell(Some(v))
                };
                if fresh || f.no_barrier {
                    fire = false;
                }
                if f.precheck && fire {
                    // Check (and log) before the store, with no re-check after it.
                    log_host(f, sc, &mut n, o);
                    n.pc = s.pc;
                    n.pending = Pending::StoreAfter(o, field, v);
                } else {
                    n.heap.get_mut(o).unwrap().insert(field, v);
                    if fire {
                        // The slow path stores, fences, then re-checks (its own step).
                        n.pending = Pending::Recheck(o);
                    }
                }
            }
        }
    }
    out.push(n);
}

/// Stop-the-world termination and the loss check. Returns the lost objects.
fn finish(f: &Form, sc: &Scenario, s: &State) -> BTreeSet<Obj> {
    let reg_cells: Vec<Obj> = s.regs.values().filter_map(|v| v.cell()).collect();
    let mut marked = s.marked.clone();
    if f.rescan {
        let mut work: Vec<Obj> = sc
            .roots
            .iter()
            .copied()
            .chain(reg_cells.iter().copied())
            .filter(|r| !marked.contains(r))
            .collect();
        if f.los_card_rescan {
            work.extend(s.dirty.iter().filter(|o| marked.contains(*o)));
        }
        marked.extend(work.iter().copied());
        while let Some(o) = work.pop() {
            for v in s.heap[o].values().filter_map(|v| v.cell()) {
                if marked.insert(v) {
                    work.push(v);
                }
            }
        }
    }
    let mut live = BTreeSet::new();
    let mut st: Vec<Obj> = sc.roots.iter().copied().chain(reg_cells).collect();
    while let Some(o) = st.pop() {
        if live.insert(o) {
            st.extend(s.heap[o].values().filter_map(|v| v.cell()));
        }
    }
    let mut kept = marked;
    if f.satb.is_some() {
        kept.extend(s.new.iter().copied()); // allocated black
    }
    live.difference(&kept).copied().collect()
}

fn run(f: &Form, sc: &Scenario) -> Outcome {
    let roots: BTreeSet<Obj> = sc.roots.iter().copied().collect();
    let init = State {
        heap: sc
            .heap
            .iter()
            .map(|(o, fs)| (*o, fs.iter().copied().collect()))
            .collect(),
        regs: BTreeMap::new(),
        pc: 0,
        marked: roots.clone(),
        black: BTreeSet::new(),
        grey: roots,
        visiting: None,
        pending: Pending::None,
        new: BTreeSet::new(),
        escaped: BTreeSet::new(),
        dirty: BTreeSet::new(),
    };
    let mut seen = HashSet::new();
    let mut stack = vec![init];
    let mut out = Outcome {
        states: 0,
        bad_end_states: 0,
        lost: BTreeSet::new(),
    };
    let mut next = Vec::new();
    while let Some(s) = stack.pop() {
        if seen.contains(&s) {
            continue;
        }
        let done = s.pc >= sc.prog.len()
            && s.pending == Pending::None
            && s.grey.is_empty()
            && s.visiting.is_none();
        if done {
            let lost = finish(f, sc, &s);
            if !lost.is_empty() {
                out.bad_end_states += 1;
                out.lost.extend(lost);
            }
        } else {
            successors(f, sc, &s, &mut next);
            stack.append(&mut next);
        }
        seen.insert(s);
    }
    out.states = seen.len();
    out
}

/// Expected (states, bad end states, lost) per scenario in `SCENARIOS` order; `None` = not run.
/// The values come from the Python models: the 9 original forms on the first 5 scenarios
/// match `model-output.txt` of the decision exactly, and the `los` column of the decided form
/// and the two single fixes match the critic's `los_card_model.out` (240 ok, 247 LOST C,
/// 247 ok). A unified Python run gave the rest.
type Row = (Form, [Option<(usize, usize, &'static str)>; 6]);

fn table() -> Vec<Row> {
    let ok = |n| Some((n, 0, ""));
    let lost = |n, b, l| Some((n, b, l));
    let rw = |name, edit: fn(&mut Form)| {
        let mut f = Form { name, ..DECIDED };
        edit(&mut f);
        f
    };
    vec![
        (
            DECIDED,
            [ok(201), ok(448), ok(29), ok(938), ok(61), ok(240)],
        ),
        (
            rw("rw_precheck", |f| f.precheck = true),
            [
                lost(213, 1, "C"),
                lost(460, 2, "CN"),
                ok(29),
                lost(951, 3, "CN"),
                lost(63, 1, "W"),
                lost(255, 1, "C"),
            ],
        ),
        (
            rw("rw_norescan", |f| f.rescan = false),
            [ok(201), ok(448), lost(29, 1, "C"), ok(938), ok(61), ok(240)],
        ),
        (
            rw("rw_oldfilter", |f| f.old_filter = true),
            [
                lost(222, 1, "C"),
                lost(436, 2, "CN"),
                ok(49),
                lost(698, 3, "CN"),
                lost(36, 1, "W"),
                lost(236, 1, "C"),
            ],
        ),
        (
            rw("rw_elide_escaped", |f| f.elide_escaped = true),
            [ok(201), ok(448), ok(29), lost(759, 1, "C"), ok(61), ok(240)],
        ),
        (
            rw("rw_nobarrier", |f| f.no_barrier = true),
            [
                lost(142, 1, "C"),
                lost(284, 2, "CN"),
                ok(29),
                lost(452, 3, "CN"),
                lost(36, 1, "W"),
                lost(158, 1, "C"),
            ],
        ),
        (
            rw("los_no_card_rescan", |f| {
                f.los_push = false;
                f.los_card_rescan = false;
            }),
            [ok(201), ok(448), ok(29), ok(938), ok(61), lost(247, 1, "C")],
        ),
        (
            rw("los_rescan_only", |f| f.los_push = false),
            [ok(201), ok(448), ok(29), ok(938), ok(61), ok(247)],
        ),
        (
            rw("los_push_only", |f| f.los_card_rescan = false),
            [ok(201), ok(448), ok(29), ok(938), ok(61), ok(240)],
        ),
        (
            satb("satb_new", Satb::New, true),
            [
                lost(142, 1, "C"),
                lost(200, 1, "C"),
                lost(29, 1, "C"),
                lost(250, 1, "C"),
                ok(58),
                None,
            ],
        ),
        (
            satb("satb_oldnew", Satb::OldNew, true),
            [ok(155), ok(236), ok(35), ok(295), ok(58), None],
        ),
        (
            satb("satb_noweakrb", Satb::OldNew, false),
            [ok(155), ok(236), ok(35), ok(295), lost(36, 1, "W"), None],
        ),
    ]
}

#[test]
#[cfg_attr(miri, ignore = "pure model, no unsafe code; slow under Miri")]
fn barrier_model_matches_the_decision() {
    let mut report = String::new();
    for (form, expect) in table() {
        for (sc, want) in SCENARIOS.iter().zip(expect) {
            let Some((states, bad, lost)) = want else {
                continue;
            };
            let got = run(&form, sc);
            let got_lost: String = got.lost.iter().copied().collect();
            report.push_str(&format!(
                "{:20} {:7} states {:4} bad {} lost {:?}\n",
                form.name, sc.name, got.states, got.bad_end_states, got_lost
            ));
            assert_eq!(
                (got.states, got.bad_end_states, got_lost.as_str()),
                (states, bad, lost),
                "form {} scenario {}\n{report}",
                form.name,
                sc.name
            );
        }
    }
    println!("{report}");
}

/// The decided form loses nothing in any scenario, and each of the five negative controls of
/// 03-gc §7 loses an object in at least one scenario.
#[test]
#[cfg_attr(miri, ignore = "pure model, no unsafe code; slow under Miri")]
fn decided_form_passes_and_negative_controls_fail() {
    let t = table();
    let form = |name: &str| t.iter().find(|(f, _)| f.name == name).unwrap().0;
    let loses = |f: &Form| SCENARIOS.iter().any(|sc| run(f, sc).bad_end_states > 0);
    assert!(!loses(&DECIDED));
    for name in [
        "rw_nobarrier",
        "rw_oldfilter",
        "rw_norescan",
        "rw_elide_escaped",
        "los_no_card_rescan",
    ] {
        assert!(
            loses(&form(name)),
            "negative control {name} must lose an object"
        );
    }
    // Either LOS fix alone is enough in the model; the decision keeps both.
    assert!(!loses(&form("los_rescan_only")) && !loses(&form("los_push_only")));
}
