//! The CORE-MATH ports keep the upstream copyright notices word for word (MIT: "The above
//! copyright notice and this permission notice shall be included"; decision M4-P1-LICENCE).
//!
//! Each line below is a copyright or author line of the C source at CORE-MATH commit
//! 1f876c544231f310f483f0a523e163850141dd7e. A `//!` line of the port's header and the
//! `THIRD-PARTY-LICENSE` "CORE-MATH" entry must both hold it (the licence file wraps lines, so
//! its whitespace is read as one space). Every CORE-MATH port is listed: MATH1's 3 (M4-P1)
//! and MATH2's 19 (M4-P4: 18 functions and `trig.rs`, the 128-bit code that `cos.c` and
//! `tan.c` share; R-MATH-P4-4). For a line that ends in an e-mail address, the MATH2 rows
//! hold the text up to the name (the header and the licence entry keep the whole line).

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
    // src/binary64/sin/sin.c
    (
        "src/sin.rs",
        &["Copyright (c) 2022-2026 Paul Zimmermann and Tom Hubrecht and Alexei Sibidanov"],
    ),
    // src/binary64/cos/cos.c
    (
        "src/cos.rs",
        &["Copyright (c) 2022-2025 Paul Zimmermann and Tom Hubrecht"],
    ),
    // src/binary64/tan/tan.c
    (
        "src/tan.rs",
        &["Copyright (c) 2022-2025 Paul Zimmermann and Tom Hubrecht"],
    ),
    // src/binary64/cos/cos.c and tan/tan.c (the shared code)
    (
        "src/trig.rs",
        &["Copyright (c) 2022-2025 Paul Zimmermann and Tom Hubrecht"],
    ),
    // src/binary64/asin/asin.c
    ("src/asin.rs", &["Copyright (c) 2022-2026 Alexei Sibidanov"]),
    // src/binary64/acos/acos.c
    (
        "src/acos.rs",
        &["Copyright (c) 2024-2025 Alexei Sibidanov."],
    ),
    // src/binary64/atan/atan.c
    ("src/atan.rs", &["Copyright (c) 2023-2026 Alexei Sibidanov"]),
    // src/binary64/atan2/{atan2.c, tint.h}
    (
        "src/atan2.rs",
        &[
            "Copyright (c) 2024-2025 Paul Zimmermann and Alexei Sibidanov",
            "Copyright (c) 2023-2024 Paul Zimmermann and Alexei Sibidanov",
        ],
    ),
    // src/binary64/sinh/sinh.c
    (
        "src/sinh.rs",
        &["Copyright (c) 2023-2026 Alexei Sibidanov and Paul Zimmermann."],
    ),
    // src/binary64/cosh/cosh.c
    ("src/cosh.rs", &["Copyright (c) 2023-2025 Alexei Sibidanov"]),
    // src/binary64/tanh/tanh.c
    (
        "src/tanh.rs",
        &["Copyright (c) 2023-2026 Alexei Sibidanov, Cyprien Peignier, Paul Zimmermann"],
    ),
    // src/binary64/asinh/asinh.c
    (
        "src/asinh.rs",
        &["Copyright (c) 2023-2025 Alexei Sibidanov"],
    ),
    // src/binary64/acosh/acosh.c
    ("src/acosh.rs", &["Copyright (c) 2023 Alexei Sibidanov."]),
    // src/binary64/atanh/atanh.c
    (
        "src/atanh.rs",
        &["Copyright (c) 2023-2025 Alexei Sibidanov"],
    ),
    // src/binary64/log2/log2.c
    ("src/log2.rs", &["Copyright (c) 2023-2026 Alexei Sibidanov"]),
    // src/binary64/log10/{log10.c, dint.h}
    (
        "src/log10.rs",
        &[
            "Copyright (c) 2022-2025 INRIA and CERN.",
            "Authors: Paul Zimmermann and Tom Hubrecht.",
            "Copyright (c) 2022 CERN.",
            "Author: Tom Hubrecht",
        ],
    ),
    // src/binary64/log1p/log1p.c
    (
        "src/log1p.rs",
        &["Copyright (c) 2024-2026 Alexei Sibidanov"],
    ),
    // src/binary64/expm1/expm1.c
    ("src/expm1.rs", &["Copyright (c) 2024 Alexei Sibidanov."]),
    // src/binary64/cbrt/cbrt.c
    (
        "src/cbrt.rs",
        &["Copyright (c) 2021-2022 Alexei Sibidanov."],
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
