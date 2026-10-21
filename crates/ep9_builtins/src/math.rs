//! Math builtins (§21.3): module `math`, block 8. Targets `%Math%` (made here:
//! `IntrinsicValue::Object`, `IntrinsicId::Math`).
//!
//! Week 4 (P3-BLTB): the constants; `abs`, `floor`, `ceil`, `trunc`, `sign`, `sqrt`, `min`, `max`,
//! `round` (`ep9_math::round_js`), `fround`, `pow` (`ep9_math::pow_js`); the global binding.
//!
//! Owner of the templates: P3-BLTB (see `TEMPLATES.md`). Ids are append-only: definition `i`
//! of [`DEFS`] is `BuiltinId::new(BLOCK, i)`, pinned by `tests/golden/ids-math.txt`.

use ep9_vm::{BuiltinDef, BuiltinModule, IntrinsicDef, ObjectProps};

/// This module's block of builtin ids.
pub const BLOCK: u8 = 8;

/// The builtin functions, in id order (append-only).
pub const DEFS: &[BuiltinDef] = &[];

/// The intrinsics this module makes.
pub const INTRINSICS: &[IntrinsicDef] = &[];

/// The properties this module adds, per object.
pub const PROPS: &[ObjectProps] = &[];

/// The module ([`crate::TABLE`] installs it).
pub const MODULE: BuiltinModule = BuiltinModule {
    name: "math",
    block: BLOCK,
    defs: DEFS,
    intrinsics: INTRINSICS,
    props: PROPS,
};
