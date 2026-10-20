//! The JSON report (schema "ep9-bench/v0"). Hand-written: the tool has no dependencies.
//!
//! ```text
//! { "schema": "ep9-bench/v0", "name": .., "command": [..], "counter": .., "unit": "instructions",
//!   "target": .., "commit": .. | null,
//!   "runs": [ { "instructions": n, "wall_ns": n, "max_rss_bytes": n | null }, .. ],
//!   "summary": { "instructions": { "min": n, "median": n, "max": n }, "wall_ns": { "median": n } } }
//! ```

use crate::counter::{Counter, Sample};
use std::fmt::Write as _;

/// The target triple this binary was built for (from build.rs).
const TARGET: &str = env!("EP9_BENCH_TARGET");

pub fn json(name: &str, command: &[String], counter: Counter, samples: &[Sample]) -> String {
    let mut o = String::new();
    o.push_str("{\n  \"schema\": \"ep9-bench/v0\",\n  \"name\": ");
    string(&mut o, name);
    o.push_str(",\n  \"command\": [");
    for (i, a) in command.iter().enumerate() {
        if i > 0 {
            o.push_str(", ");
        }
        string(&mut o, a);
    }
    o.push_str("],\n  \"counter\": ");
    string(&mut o, counter.id());
    o.push_str(",\n  \"unit\": \"instructions\",\n  \"target\": ");
    string(&mut o, TARGET);
    o.push_str(",\n  \"commit\": ");
    // CI sets GITHUB_SHA; local runs may set EP9_COMMIT.
    match std::env::var("GITHUB_SHA").or_else(|_| std::env::var("EP9_COMMIT")) {
        Ok(sha) => string(&mut o, &sha),
        Err(_) => o.push_str("null"),
    }
    o.push_str(",\n  \"runs\": [\n");
    for (i, s) in samples.iter().enumerate() {
        let rss = s
            .max_rss_bytes
            .map_or_else(|| "null".to_string(), |b| b.to_string());
        let _ = write!(
            o,
            "    {{ \"instructions\": {}, \"wall_ns\": {}, \"max_rss_bytes\": {rss} }}",
            s.instructions, s.wall_ns
        );
        o.push_str(if i + 1 < samples.len() { ",\n" } else { "\n" });
    }
    let mut ins: Vec<u64> = samples.iter().map(|s| s.instructions).collect();
    let mut wall: Vec<u128> = samples.iter().map(|s| s.wall_ns).collect();
    ins.sort_unstable();
    wall.sort_unstable();
    let _ = write!(
        o,
        "  ],\n  \"summary\": {{\n    \"instructions\": {{ \"min\": {}, \"median\": {}, \"max\": {} }},\n    \
         \"wall_ns\": {{ \"median\": {} }}\n  }}\n}}\n",
        ins[0],
        median_u64(&ins),
        ins[ins.len() - 1],
        median_u128(&wall),
    );
    o
}

/// Median of a sorted, non-empty slice; the lower middle for an even length.
fn median_u64(sorted: &[u64]) -> u64 {
    sorted[(sorted.len() - 1) / 2]
}

fn median_u128(sorted: &[u128]) -> u128 {
    sorted[(sorted.len() - 1) / 2]
}

/// Append `s` as a JSON string literal.
fn string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes() {
        let mut s = String::new();
        string(&mut s, "a\"b\\c\n\u{1}");
        assert_eq!(s, r#""a\"b\\c\n\u0001""#);
    }

    #[test]
    fn medians() {
        assert_eq!(median_u64(&[1, 2, 3]), 2);
        assert_eq!(median_u64(&[1, 2, 3, 4]), 2);
        assert_eq!(median_u64(&[7]), 7);
    }

    #[test]
    fn report_shape() {
        let samples = [
            Sample {
                instructions: 30,
                wall_ns: 3,
                max_rss_bytes: Some(1),
            },
            Sample {
                instructions: 10,
                wall_ns: 1,
                max_rss_bytes: None,
            },
            Sample {
                instructions: 20,
                wall_ns: 2,
                max_rss_bytes: None,
            },
        ];
        let j = json("x", &["ep9".into(), "-e".into()], Counter::Perf, &samples);
        assert!(j.contains("\"schema\": \"ep9-bench/v0\""));
        assert!(j.contains("\"command\": [\"ep9\", \"-e\"]"));
        assert!(j.contains("\"counter\": \"perf-stat-instructions-u\""));
        assert!(j.contains("\"instructions\": { \"min\": 10, \"median\": 20, \"max\": 30 }"));
        assert!(j.contains("\"max_rss_bytes\": null"));
        assert!(j.contains("\"wall_ns\": { \"median\": 2 }"));
    }
}
