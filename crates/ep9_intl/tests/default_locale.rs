//! DefaultLocale reads the host locale once per process (Codex r1 COLLATOR-2). Before, the
//! default collator of `localeCompare` (a thread-local cache slot, made once) kept the first
//! host locale, while `Intl.Collator()` read the host again: after LC_ALL changed, the two
//! gave different orders on one thread. This file has one test, so no other thread reads
//! the environment while it changes (env vars are per process).

mod common;

#[cfg(unix)]
#[test]
fn a_host_locale_change_after_the_first_use_changes_nothing() {
    let probe = r#"print("ä".localeCompare("z"), new Intl.Collator().compare("ä", "z"), "ä".localeCompare("z", undefined, {}), new Intl.Collator().resolvedOptions().locale);"#;
    // SAFETY: this test binary runs one test, and no other thread of it reads or writes the
    // environment while the variable changes.
    unsafe { std::env::set_var("LC_ALL", "de_DE.UTF-8") };
    assert_eq!(
        common::run(probe, false),
        common::lines(&["-1 -1 -1 de-DE"])
    );
    // SAFETY: as above.
    unsafe { std::env::set_var("LC_ALL", "sv_SE.UTF-8") };
    // A new isolate on the same thread: Swedish sorts "ä" after "z", but the default locale
    // stays German for every path.
    assert_eq!(
        common::run(probe, false),
        common::lines(&["-1 -1 -1 de-DE"])
    );
}
