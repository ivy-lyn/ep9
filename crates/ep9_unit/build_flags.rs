//! The build-flag rule of MC-w28-9.1 (doc 13 C-19, the A2 build rule): a build of `ep9_unit`
//! or of `ep9_vm` (so of the `ep9` engine and all that links it) does not opt in to a hardware
//! shadow stack (x86-64 CET, AArch64 GCS). Lazy deopt replaces a native return address in a
//! frame record (the H29 Contract: "lazy deopt by return-address swap only"), and a shadow
//! stack faults on that return. The H29 A4 part decides with data; until then both build
//! scripts refuse a flag that turns one on (SHSTK-ENGINE, M7-P4: `ep9_vm` holds
//! `deopt::swap_returns`, and `crates/ep9_vm/build.rs` runs this check too).
//! `build.rs`, `tests/build_flags.rs` and `../ep9_vm/build.rs` include this file with `#[path]`.
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
    // The linker words of every link-arg item, in order, each with its item: the driver's
    // forwarding words are dropped (`-Wl,` splits at commas into the same words; `-Xlinker`),
    // so an option split over two items (`-Clink-arg=-Wl,-z -Clink-arg=-Wl,shstk`) is seen
    // whole (Codex r1 A2-START 2).
    let mut words: Vec<(usize, String)> = Vec::new();
    for (n, item) in items.iter().enumerate() {
        if let Some(v) = link_args(&item.to_ascii_lowercase()) {
            for w in v.split([' ', ',']) {
                if !w.is_empty() && w != "-wl" && w != "-xlinker" {
                    words.push((n, w.to_string()));
                }
            }
        }
    }
    // The item of the first linker opt-in: `-z shstk`, `-zshstk` or `/CETCOMPAT`.
    let link_opt_in = words.iter().enumerate().find_map(|(k, (n, w))| {
        let shstk =
            w == "-zshstk" || (w == "-z" && words.get(k + 1).is_some_and(|(_, x)| x == "shstk"));
        (shstk || w.trim_start_matches(['/', '-']) == "cetcompat").then_some(*n)
    });
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
        if link_opt_in == Some(n) {
            return Some(item.clone());
        }
    }
    None
}
