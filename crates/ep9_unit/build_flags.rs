//! The build-flag rule of MC-w28-9.1 (doc 13 C-19, the A2 build rule): an ep9 build does not
//! opt in to a hardware shadow stack (x86-64 CET, AArch64 GCS). Lazy deopt replaces a native
//! return address in a frame record (the H29 Contract: "lazy deopt by return-address swap
//! only"), and a shadow stack faults on that return. The H29 A4 part decides with data; until
//! then `build.rs` refuses a flag that turns one on. `build.rs` and `tests/build_flags.rs` both
//! include this file with `#[path]`.
//!
//! ```text
//! -Zcf-protection=return | full        rustc: x86-64 CET shadow stack (IBT alone is "branch")
//! -Zbranch-protection=...gcs...        rustc: AArch64 Guarded Control Stack
//! -Clink-arg(s)=.../CETCOMPAT...       link.exe: mark the Windows image CET-compatible
//! -Clink-arg(s)=...-z shstk / -zshstk  ELF linkers: the shadow-stack property
//! ```

/// The first flag of `flags` (rustc arguments, one per item, as `CARGO_ENCODED_RUSTFLAGS`
/// splits them) that opts in to a hardware shadow stack, or None. A flag and its value may be
/// one item (`-Zcf-protection=full`, `-Clink-arg=/CETCOMPAT`) or two (`-Z cf-protection=full`,
/// `-C link-arg=/CETCOMPAT`).
pub fn shadow_stack_opt_in(flags: &[&str]) -> Option<String> {
    let mut items: Vec<String> = Vec::new();
    let mut i = 0;
    while i < flags.len() {
        let f = flags[i];
        // `-Z x` and `-C x` as two items: join them.
        if (f == "-Z" || f == "-C") && i + 1 < flags.len() {
            items.push(format!("{f}{}", flags[i + 1]));
            i += 2;
            continue;
        }
        items.push(f.to_string());
        i += 1;
    }
    // On a lowercase item: the value of `-Clink-arg=` or `-Clink-args=`.
    let link_args = |s: &str| -> Option<String> {
        let v = s
            .strip_prefix("-clink-arg=")
            .or_else(|| s.strip_prefix("-clink-args="))?;
        Some(v.to_string())
    };
    for (n, item) in items.iter().enumerate() {
        let lower = item.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("-zcf-protection=")
            && (v == "return" || v == "full")
        {
            return Some(item.clone());
        }
        if let Some(v) = lower.strip_prefix("-zbranch-protection=")
            && v.split(',').any(|p| p == "gcs")
        {
            return Some(item.clone());
        }
        if let Some(v) = link_args(&lower) {
            let words: Vec<&str> = v.split([' ', ',']).filter(|w| !w.is_empty()).collect();
            let shstk = words.windows(2).any(|w| w[0] == "-z" && w[1] == "shstk")
                || words.contains(&"-zshstk")
                || (words.last() == Some(&"-z")
                    && items
                        .get(n + 1)
                        .and_then(|x| link_args(&x.to_ascii_lowercase()))
                        .is_some_and(|x| x == "shstk"));
            if shstk
                || words
                    .iter()
                    .any(|w| w.trim_start_matches(['/', '-']) == "cetcompat")
            {
                return Some(item.clone());
            }
        }
    }
    None
}
