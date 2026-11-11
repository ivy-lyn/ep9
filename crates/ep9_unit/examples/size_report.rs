//! The SIZE-REPORT (doc 13 C-17, K-A3; MODES-CHECK MC-w28-8.1): for each workload, the bytes
//! of a unit under the 3 coverage policies of `ep9_unit::size`, with the deopt metadata
//! bytes beside the text bytes.
//!
//! ```text
//! ep9 --facts-out w.facts w.js          (a training run: the PGO set)
//! cargo run -p ep9_unit --example size_report -- w.js w.facts [x.js x.facts ...]
//! ```
//!
//! Each workload is compiled as `ep9` compiles a script (sloppy, default options), so its
//! content hash names the facts file's unit; a facts file of another unit is refused.

use std::fmt::Write as _;

use ep9_bytecode::{Instructions, Opcode};
use ep9_frontend::{CompileOptions, SourceOpts, Strictness};
use ep9_unit::size::{FunctionStats, Policy, SAMPLE, size};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || !args.len().is_multiple_of(2) {
        eprintln!("usage: size_report <file.js> <file.facts> [...]");
        std::process::exit(2);
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "SIZE-REPORT (est. text {} B and metadata {} B per {} bytecode instructions: Spike C am3)",
        SAMPLE.1, SAMPLE.2, SAMPLE.0
    );
    let _ = writeln!(
        out,
        "{:<14} {:>5} {:>8} | {:>7} {:>7} {:>9} {:>9} {:>9} {:>8} | per policy",
        "workload", "fns", "bc B", "policy", "fns", "table B", "text B", "meta B", "meta %"
    );
    let mut totals = [(0u64, 0u64, 0u64, 0u64); 3];
    for pair in args.chunks(2) {
        let (js, facts) = (&pair[0], &pair[1]);
        let source = std::fs::read_to_string(js).unwrap_or_else(|e| panic!("{js}: {e}"));
        let unit = ep9_frontend::compile(
            &source,
            SourceOpts::script(Strictness::Sloppy),
            CompileOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{js}: {e:?}"))
        .into_inner();
        let bytes = std::fs::read(facts).unwrap_or_else(|e| panic!("{facts}: {e}"));
        let snap =
            ep9_facts::FactSnapshot::decode(&bytes).unwrap_or_else(|e| panic!("{facts}: {e}"));
        assert_eq!(
            snap.header().unit_hash,
            unit.content_hash(),
            "{facts} holds the facts of another unit than {js}"
        );
        let warm: std::collections::BTreeSet<u32> =
            snap.records().map(|(k, _)| k.function).collect();
        let fns: Vec<FunctionStats> = unit
            .functions
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let insns: Vec<_> = Instructions::new(&f.bytecode)
                    .map(|r| r.expect("verified bytecode"))
                    .collect();
                FunctionStats {
                    index: i as u32,
                    instructions: insns.len() as u32,
                    bytecode_bytes: f.bytecode.len() as u32,
                    has_loop: insns.iter().any(|x| x.opcode == Opcode::JumpLoop),
                    in_profile: warm.contains(&(i as u32)),
                }
            })
            .collect();
        let bc: u64 = fns.iter().map(|f| u64::from(f.bytecode_bytes)).sum();
        let name = std::path::Path::new(js)
            .file_stem()
            .map_or_else(|| js.clone(), |s| s.to_string_lossy().into_owned());
        for (k, p) in Policy::ALL.into_iter().enumerate() {
            let s = size(&fns, p);
            let native = s.native_bytes();
            let pct = if native == 0 {
                0.0
            } else {
                100.0 * s.meta_bytes as f64 / native as f64
            };
            let (w, nf, nb) = if k == 0 {
                (name.as_str(), fns.len().to_string(), bc.to_string())
            } else {
                ("", String::new(), String::new())
            };
            let _ = writeln!(
                out,
                "{w:<14} {nf:>5} {nb:>8} | {:>7} {:>7} {:>9} {:>9} {:>9} {pct:>7.1}%",
                p.name(),
                s.functions,
                s.table_bytes,
                s.text_bytes,
                s.meta_bytes
            );
            let t = &mut totals[k];
            *t = (
                t.0 + u64::from(s.functions),
                t.1 + s.table_bytes,
                t.2 + s.text_bytes,
                t.3 + s.meta_bytes,
            );
        }
    }
    for (k, p) in Policy::ALL.into_iter().enumerate() {
        let (f, t, x, m) = totals[k];
        let _ = writeln!(
            out,
            "{:<14} {:>5} {:>8} | {:>7} {f:>7} {t:>9} {x:>9} {m:>9} {:>7.1}%",
            if k == 0 { "total" } else { "" },
            "",
            "",
            p.name(),
            100.0 * m as f64 / (t + x + m).max(1) as f64
        );
    }
    print!("{out}");
}
