//! The facts input of the graph builder: one FactSnapshot, from one of two paths (H30 rule
//! "one graph builder reads bytecode + FactSnapshot"; V9-D32, JA-D11).
//!
//! ```text
//!  live (Spike C):   T0 run ──ep9_vm::abi::export_facts──> FactSnapshot (source RUNTIME)
//!  SP-Facts:         training run ──export_facts──> to_profiled ──encode──> file
//!                    file ──read_profiled (decode, source PROFILED, validate vs the unit)──>
//!                    FactSnapshot (source PROFILED)
//!  both:             for_function(snapshot, function id) ──> FunctionFacts ──> build
//! ```
//!
//! The `.ep9prof` container is H16-Q5b (A3); until then the SP-Facts file is the plain
//! FactSnapshot encoding with the source byte PROFILED ([`to_profiled`]), the form the
//! protocol (§6) names.

use ep9_bytecode::{CompiledUnit, FeedbackSlotKind};
use ep9_facts::{
    BinaryHint, CompareHint, Dropped, FactHeader, FactSnapshot, FactSource, SlotFacts,
};

use crate::harness::{Script, Session};

/// The facts of one function, in slot order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionFacts<'a> {
    /// The snapshot's source (RUNTIME for the live path, PROFILED for SP-Facts).
    pub source: FactSource,
    /// The stable function id (preorder, H2).
    pub function: u32,
    /// `(slot index, facts)` of each slot that has a record.
    pub records: Vec<(u32, &'a SlotFacts)>,
}

impl FunctionFacts<'_> {
    /// No facts (every value of the graph is then `unknown` or PROVEN).
    pub fn none(source: FactSource, function: u32) -> FunctionFacts<'static> {
        FunctionFacts {
            source,
            function,
            records: Vec::new(),
        }
    }
}

/// The live facts (source RUNTIME) of the unit of `script`, from the feedback of `session`'s
/// isolate (`ep9_vm::abi::export_facts`).
pub fn live(session: &Session, script: &Script) -> Result<FactSnapshot, String> {
    ep9_vm::abi::export_facts(session.iso(), &script.unit.content_hash())
        .map_err(|e| format!("{}: export_facts: {e}", script.file))
}

/// The same records under the source PROFILED: the SP-Facts file of the protocol (§6) until
/// the `.ep9prof` container exists (H16-Q5b).
pub fn to_profiled(snapshot: &FactSnapshot) -> Result<FactSnapshot, String> {
    let h = snapshot.header();
    let mut out = FactSnapshot::new(FactHeader {
        source: FactSource::Profiled,
        ..*h
    });
    for (k, f) in snapshot.records() {
        out.insert(k.function, k.slot, f.clone())
            .map_err(|e| format!("record {}/{}: {e:?}", k.function, k.slot))?;
    }
    Ok(out)
}

/// Reads a PROFILED file for `unit`: decodes it, refuses any source other than PROFILED,
/// and checks it against the unit (`FactSnapshot::validate`: a stale snapshot is refused, a
/// record that does not fit the unit is dropped and listed).
pub fn read_profiled(
    bytes: &[u8],
    unit: &CompiledUnit,
) -> Result<(FactSnapshot, Vec<Dropped>), String> {
    let s = FactSnapshot::decode(bytes).map_err(|e| format!("decode: {e:?}"))?;
    if s.header().source != FactSource::Profiled {
        return Err(format!(
            "the file's source is {:?}, not Profiled",
            s.header().source
        ));
    }
    s.validate(unit).map_err(|e| format!("stale: {e:?}"))
}

/// `snapshot` without the BinaryOp and CompareOp records of `function`: the tests' "no int
/// fact" baseline, now that the live facts hold those records (FACTS2, M4-P2).
pub fn without_int(snapshot: &FactSnapshot, function: u32) -> FactSnapshot {
    let mut out = snapshot.clone();
    let int: Vec<u32> = snapshot
        .records()
        .filter(|(k, f)| {
            k.function == function && matches!(f, SlotFacts::BinaryOp(_) | SlotFacts::CompareOp(_))
        })
        .map(|(k, _)| k.slot)
        .collect();
    for slot in int {
        out.remove(function, slot);
    }
    out
}

/// A test stand-in for the int facts FACTS2 exports (M4-P2): `snapshot` plus, for each
/// BinaryOp and CompareOp slot of `function` in `unit` that has no record, the record the T0
/// profile's reader rule gives a slot that T0 left empty (`ep9_vm` profile.rs, "Implicit
/// int32": the reader adds `INT32` to every slot of its kind in a function that has a
/// vector). Only the spike's tests use it; since FACTS2 the live facts hold these records
/// (`facts_export.rs` adds the INT32 bit to each, so a slot T0 left empty reads INT32).
pub fn with_implicit_int32(
    snapshot: &FactSnapshot,
    unit: &CompiledUnit,
    function: u32,
) -> Result<FactSnapshot, String> {
    let mut out = snapshot.clone();
    let f = unit
        .functions
        .get(function as usize)
        .ok_or_else(|| format!("no function {function}"))?;
    for (slot, kind) in f.feedback_slots.iter().enumerate() {
        let slot = slot as u32;
        if out.get(function, slot).is_some() {
            continue;
        }
        let rec = match kind {
            FeedbackSlotKind::BinaryOp => SlotFacts::BinaryOp(BinaryHint::INT32),
            FeedbackSlotKind::CompareOp => SlotFacts::CompareOp(CompareHint::INT32),
            _ => continue,
        };
        out.insert(function, slot, rec)
            .map_err(|e| format!("record {function}/{slot}: {e:?}"))?;
    }
    Ok(out)
}

/// The records of function `function` in `snapshot`.
pub fn for_function(snapshot: &FactSnapshot, function: u32) -> FunctionFacts<'_> {
    FunctionFacts {
        source: snapshot.header().source,
        function,
        records: snapshot
            .records()
            .filter(|(k, _)| k.function == function)
            .map(|(k, f)| (k.slot, f))
            .collect(),
    }
}

/// The record kind name of `f` (reports).
pub fn kind_name(f: &SlotFacts) -> String {
    let s = format!("{f:?}");
    s.split(['(', ' ', '{']).next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use spike_b::harness::Config;

    /// A loop with named loads (the kinds T0 exports today: LoadNamed, LoadGlobal) runs past
    /// the feedback threshold.
    const SRC: &str = "\
function P(x) { this.x = x; this.y = x & 7; }\n\
function sum(ps) { var s = 0; for (var i = 0; i < ps.length; i++) { s = (s + ps[i].x + ps[i].y) | 0; } return s; }\n\
var ps = []; for (var i = 0; i < 50; i++) ps.push(new P(i));\n\
var r = 0; for (var k = 0; k < 40; k++) r = sum(ps);\n\
print(r);\n";

    #[test]
    fn live_then_profiled_round_trip() {
        let mut s = Session::new(&Config::default()).expect("session");
        let r = s.run(SRC, "f.js");
        assert_eq!(r.output, ["1394"], "{r:?}");
        let script = s.script("f.js").expect("script");
        let sum = script.functions_named("sum");
        assert_eq!(sum.len(), 1);
        let live = live(&s, script).expect("live facts");
        assert_eq!(live.header().source, FactSource::Runtime);
        let f = for_function(&live, sum[0]);
        assert_eq!(f.source, FactSource::Runtime);
        assert!(
            f.records.iter().any(|(_, r)| kind_name(r) == "LoadNamed"),
            "{:?}",
            f.records
        );
        // The PROFILED file: the same records, the source byte changed, and it reads back
        // against the same unit with nothing dropped.
        let prof = to_profiled(&live).expect("profiled");
        let bytes = prof.encode();
        let (back, dropped) = read_profiled(&bytes, script.unit.unit()).expect("read");
        assert!(dropped.is_empty(), "{dropped:?}");
        assert_eq!(back, prof);
        assert_eq!(back.header().source, FactSource::Profiled);
        let g = for_function(&back, sum[0]);
        assert_eq!(g.records, f.records);
        // A RUNTIME file is not a PROFILED file.
        let e = read_profiled(&live.encode(), script.unit.unit()).expect_err("runtime");
        assert!(e.contains("Runtime"), "{e}");
        // Another unit: stale.
        s.run("print(1);", "other.js");
        let other = s.script("other.js").expect("other");
        assert!(
            read_profiled(&bytes, other.unit.unit())
                .expect_err("stale")
                .starts_with("stale")
        );
    }

    /// Why keyed records back no element guard (SPIKEC-3): in FACT_SCHEMA 1 a LoadKeyed site
    /// whose reads went out of bounds, hit a hole or had a non-array receiver exports the
    /// same keys, {index}, as a site that only read present elements (the receiver, bounds
    /// and hole bits of the slot have no schema field, `ep9_vm` facts_export.rs `keyed`).
    #[test]
    fn keyed_records_do_not_show_bounds_holes_or_receivers() {
        use ep9_facts::KeySet;
        let mut s = Session::new(&Config::default()).expect("session");
        let src = "\
function present(a, i) { return a[i]; }\n\
function oob(a, i) { return a[i]; }\n\
function hole(a, i) { return a[i]; }\n\
function obj(a, i) { return a[i]; }\n\
function name(a, i) { return a[i]; }\n\
var xs = [1, 2, 3], hs = [1, , 3], o = { 0: 1, 1: 2 };\n\
for (var k = 0; k < 30; k++) { present(xs, k % 3); oob(xs, 3 + k); hole(hs, 1); obj(o, k & 1); name(xs, 'length'); }\n";
        assert!(s.run(src, "k.js").result.is_ok());
        let script = s.script("k.js").expect("script");
        let live = live(&s, script).expect("live");
        let keys = |f: &str| {
            let id = script.functions_named(f)[0];
            let recs = for_function(&live, id).records;
            let [(_, SlotFacts::LoadKeyed(k))] = recs[..] else {
                panic!("{f}: {recs:?}");
            };
            k.keys
        };
        for f in ["present", "oob", "hole", "obj"] {
            assert_eq!(keys(f), KeySet::INDEX, "{f}");
        }
        assert_eq!(keys("name"), KeySet::INDEX.union(KeySet::NAME));
    }

    #[test]
    fn none_has_no_records() {
        let f = FunctionFacts::none(FactSource::Profiled, 3);
        assert!(f.records.is_empty());
        assert_eq!(f.function, 3);
    }
}
