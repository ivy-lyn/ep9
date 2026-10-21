# fp-check fixtures

Mini workspaces for `tools/xtask/tests/fpcheck.rs`. A test copies `common/` (the workspace and the
`fx_cli` binary) and then one case directory (the `fx_engine` crate) into a temp dir, and runs
`cargo xtask fp-check --root <copy> -p fx_cli --bin fx --target ...` there. Never build them in
place: each copy is its own workspace (`[workspace]` in `common/Cargo.toml`).

| Case | What `fx_engine` does | fp-check must |
|---|---|---|
| `float-rem` | `a % b` on f64 (lowers to the C `fmod`) | exit 1 naming `fx_engine::js_remainder`; exit 0 with an allow entry for it |
| `mul-add` | `f64::mul_add` outside ep9_math | exit 1 naming `fx_engine::fused_step`: a `vfmadd` under `x86-64-v3`, the C `fma` on the x86-64 baseline |
| `libm-import` | calls `cbrt` through `extern "C"` (a libm import without the libm crate) | exit 1 naming `fx_engine::fake_libm_cbrt` |
| `allow-site` | `#[allow(clippy::disallowed_methods)]` outside `crates/ep9_math/src/exact.rs` | exit 1 naming `fx_engine::allowed_sqrt` |
| `ep9-math-user` | `ep9_math::fma_exact` and `two_prod` (the test fills in the path to `crates/ep9_math`) | exit 0, with the FMA instructions inside `ep9_math::` (on the x86-64 baseline in the dispatch's FMA copy) and no `fma` call |
| `clean` | plain IEEE arithmetic | exit 0 (the control) |
