#![allow(dead_code)] // m5-stub
//! `cargo xtask extref-lint` (plan v10 C-26, V10-A7; H25-Q2 as H25:85 words it): native words
//! in heap cells go through `ExternalRef<T>`. The lint will reject `native_of` and `native_ptr`
//! outside `ExternalRef` and the VM stack code (the allow list of H25:54, by file and line), and
//! a native word copied from one heap-cell field into another.
//!
//! M5-P0 (GATES5) adds only the task name, because `tools/xtask/src/main.rs` is frozen after
//! M5-P0 (plan v10 rule 2); M1A-CODE writes the body in M5-P3. Until then the task exits 2,
//! "not built yet", as every task that is not implemented yet does.

/// The task: always an error until M5-P3.
pub fn main(_args: &[String]) -> Result<bool, String> {
    Err("not built yet (plan v10: M1A-CODE writes the lint in M5-P3)".to_string())
}

#[cfg(test)]
mod tests {
    /// The stub refuses to pass: exit 2 through `main`'s `Err` arm.
    #[test]
    fn the_stub_is_not_built_yet() {
        let e = super::main(&[]).unwrap_err();
        assert!(e.starts_with("not built yet"), "{e}");
    }
}
