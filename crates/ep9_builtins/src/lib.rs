//! ECMA-262 builtins written against the builtin ABI (H20). The crate's engine dependencies are
//! `ep9_vm` and `ep9_math` only (`.github/scripts/dep-guards.sh` check 8), and it has no
//! `unsafe` code.
#![forbid(unsafe_code)]
