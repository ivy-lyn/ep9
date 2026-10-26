//! The CORE-MATH ports keep the upstream copyright notices word for word (MIT: "The above
//! copyright notice and this permission notice shall be included"; decision M4-P1-LICENCE).
//!
//! Each line below is a copyright or author line of the C source at CORE-MATH commit
//! 1f876c544231f310f483f0a523e163850141dd7e. A `//!` line of the port's header and the
//! `THIRD-PARTY-LICENSE` "CORE-MATH" entry must both hold it (the licence file wraps lines, so
//! its whitespace is read as one space).

use std::path::Path;

/// (ported file, the upstream notice lines it must carry).
const NOTICES: &[(&str, &[&str])] = &[
    // src/binary64/exp/exp.c
    (
        "src/exp.rs",
        &["Copyright (c) 2022-2026 Alexei Sibidanov <sibid@uvic.ca>."],
    ),
    // src/binary64/log/log.c, src/binary64/log/dint.h
    (
        "src/log.rs",
        &[
            "Copyright (c) 2022 INRIA and CERN.",
            "Authors: Paul Zimmermann and Tom Hubrecht.",
            "Copyright (c) 2022 CERN.",
            "Author: Tom Hubrecht",
        ],
    ),
    // src/binary64/pow/{pow.c, pow.h, dint.h, qint.h}
    (
        "src/pow.rs",
        &[
            "Copyright (c) 2022-2025 CERN and Inria",
            "Copyright (c) 2022, 2023 CERN and Inria",
            "Authors: Tom Hubrecht and Paul Zimmermann",
        ],
    ),
];

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn one_space(t: &str) -> String {
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
// Miri runs with isolation on, so it cannot open the source files this test reads.
#[cfg_attr(miri, ignore)]
fn core_math_ports_keep_the_upstream_notices() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let licence = one_space(&read(&dir.join("../../THIRD-PARTY-LICENSE")));
    for (file, lines) in NOTICES {
        let src = read(&dir.join(file));
        for line in *lines {
            assert!(
                src.lines()
                    .any(|l| l.starts_with("//!") && l.contains(line)),
                "{file}: no header line {line:?}"
            );
            assert!(
                licence.contains(line),
                "THIRD-PARTY-LICENSE: no {line:?} (for {file})"
            );
        }
    }
}
