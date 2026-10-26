#![allow(dead_code)] // m4-stub
//! The strict `eval`/`arguments` post-filter for oxc_semantic diagnostics (PRE-1).
//!
//! Not built yet: the FE-FUZZ role (M4-P2) fills this file. oxc_semantic 0.155.0 reports the
//! strict-mode "cannot assign to `eval` or `arguments`" error for some forms where the
//! identifier is read, not assigned (for example a default value in a destructuring pattern).
//! The filter will drop that diagnostic when its IdentifierReference is not itself the
//! assignment target; `semantic` in lib.rs and the semantic step of eval.rs will call it. Real
//! targets (`[eval] = []`, `({eval} = {})`, `eval++`) keep their SyntaxError.
