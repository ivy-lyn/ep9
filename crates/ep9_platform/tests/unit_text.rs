//! `unit_text` as other crates see it (the library built without `cfg(test)`): in a build
//! without `jit` the process code range is a plain reservation on every target, so unit text
//! takes the protect form (commit RW, copy, icache flush, protect RX; no `MAP_JIT`). With
//! `--features jit` it takes the window form. DL M6-ORCH-298; H28 unit mapping.

use ep9_platform::unit_text::{Form, UnitText};
use ep9_platform::{CodeRange, page_size};

/// `return x + 7` (u64 in, u64 out) in the C ABI of the host (no stack use).
fn add7() -> Vec<u8> {
    #[cfg(target_arch = "aarch64")]
    {
        // add x0, x0, #7; ret
        [0x9100_1C00u32, 0xD65F_03C0]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect()
    }
    #[cfg(all(target_arch = "x86_64", not(windows)))]
    {
        // lea rax, [rdi + 7]; ret
        vec![0x48, 0x8D, 0x47, 0x07, 0xC3]
    }
    #[cfg(all(target_arch = "x86_64", windows))]
    {
        // lea rax, [rcx + 7]; ret
        vec![0x48, 0x8D, 0x41, 0x07, 0xC3]
    }
}

/// The form is the build's (no `jit`: protect, never `MAP_JIT`), the bytes are the text, and
/// the code runs from the process code range.
#[test]
fn unit_text_runs_in_the_process_code_range() {
    let range = CodeRange::process().unwrap();
    let code = add7();
    let t = UnitText::map(range, &code).unwrap();
    if cfg!(feature = "jit") {
        assert_eq!(t.form(), Form::Window);
    } else {
        assert!(
            !range.is_map_jit(),
            "a build without jit maps no MAP_JIT range"
        );
        assert_eq!(t.form(), Form::Protect);
    }
    assert_eq!(t.len(), code.len());
    assert!(range.contains(t.base()) && range.contains(t.range().end - 1));
    assert!(t.base().is_multiple_of(page_size()));
    // SAFETY: the text is mapped readable and executable until `t` drops.
    let bytes = unsafe { core::slice::from_raw_parts(t.base() as *const u8, t.len()) };
    assert_eq!(bytes, &code[..]);
    // SAFETY: the text is `add7`, mapped executable until `t` drops.
    let f: extern "C" fn(u64) -> u64 = unsafe { core::mem::transmute(t.base()) };
    assert_eq!(f(35), 42);
}

/// Two texts get two chunks; each drop gives its pages back, and a later text can reuse them.
#[test]
fn two_texts_do_not_share_pages() {
    let range = CodeRange::process().unwrap();
    let a = UnitText::map(range, &add7()).unwrap();
    let b = UnitText::map(range, &vec![0u8; page_size() + 1]).unwrap();
    assert_eq!(b.pages().len(), 2 * page_size());
    assert!(a.pages().end <= b.pages().start || b.pages().end <= a.pages().start);
    assert!(!a.contains(b.base()) && !b.contains(a.base()));
    drop(b);
    // SAFETY: `a` is still mapped.
    let f: extern "C" fn(u64) -> u64 = unsafe { core::mem::transmute(a.base()) };
    assert_eq!(f(0), 7);
}
