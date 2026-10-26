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

use ep9_bytecode::CompiledUnit;
use ep9_facts::{Dropped, FactHeader, FactSnapshot, FactSource, SlotFacts};

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

    #[test]
    fn none_has_no_records() {
        let f = FunctionFacts::none(FactSource::Profiled, 3);
        assert!(f.records.is_empty());
        assert_eq!(f.function, 3);
    }
}
