//! C-23 (M5-P0 P0-VM5): `ExecOptions`, `Isolate::new_with` and `build_info` (`exec.rs`).
//! ep9_vm's own tests build without the `jit` feature unless a run turns it on, so the
//! M-R6 test checks the refusal or the acceptance by the build's features.

use crate::{BuildInfo, ExecOptions, Feedback, Isolate, IsolateConfig, IsolateError, Preset};

#[test]
fn presets_have_their_default_feedback_and_names() {
    let jit = ExecOptions::jit();
    assert_eq!(
        (jit.preset(), jit.feedback_mode()),
        (Preset::JIT, Feedback::FULL)
    );
    let jitless = ExecOptions::jitless();
    assert_eq!(
        (jitless.preset(), jitless.feedback_mode()),
        (Preset::JITLESS, Feedback::ICS_ONLY)
    );
    for p in [Preset::JIT, Preset::JITLESS] {
        assert_eq!(Preset::from_name(p.name()), Some(p));
    }
    for f in [Feedback::FULL, Feedback::ICS_ONLY] {
        assert_eq!(Feedback::from_name(f.name()), Some(f));
    }
    assert_eq!(
        (Preset::JIT.name(), Preset::JITLESS.name()),
        ("jit", "jitless")
    );
    assert_eq!(
        (Feedback::FULL.name(), Feedback::ICS_ONLY.name()),
        ("full", "ics_only")
    );
    // A name no build assigns (doc 13 §5.4: never the next planned one).
    assert_eq!(Preset::from_name("x-never"), None);
    assert_eq!(Feedback::from_name("none"), None);
}

#[test]
fn the_default_options_follow_the_jit_feature() {
    let want = if cfg!(feature = "jit") {
        ExecOptions::jit()
    } else {
        ExecOptions::jitless()
    };
    assert_eq!(ExecOptions::default(), want);
    assert_eq!(ExecOptions::build_default(), want);
    let iso = Isolate::new(IsolateConfig::default()).unwrap();
    assert_eq!(iso.rt.exec.get(), want);
}

/// M-R6 (doc 13:77): a preset the build cannot run is `IsolateError::Config`.
#[test]
fn the_jit_preset_needs_the_jit_feature() {
    let r = Isolate::new_with(IsolateConfig::default(), ExecOptions::jit());
    if cfg!(feature = "jit") {
        assert!(Preset::JIT.available());
        assert_eq!(r.unwrap().rt.exec.get(), ExecOptions::jit());
    } else {
        assert!(!Preset::JIT.available());
        match r {
            Err(IsolateError::Config(m)) => assert!(m.contains("jit feature"), "{m}"),
            Err(e) => panic!("another error: {e}"),
            Ok(_) => panic!("a build without jit ran the jit preset"),
        }
    }
    assert!(Preset::JITLESS.available());
}

/// The feedback setter works with both presets (V10-A20); the isolate stores it.
#[test]
fn the_feedback_setter_works_with_both_presets() {
    let presets = if cfg!(feature = "jit") {
        vec![ExecOptions::jit(), ExecOptions::jitless()]
    } else {
        vec![ExecOptions::jitless()]
    };
    for base in presets {
        for f in [Feedback::FULL, Feedback::ICS_ONLY] {
            let exec = base.feedback(f);
            assert_eq!((exec.preset(), exec.feedback_mode()), (base.preset(), f));
            let iso = Isolate::new_with(IsolateConfig::default(), exec).unwrap();
            assert_eq!(iso.rt.exec.get(), exec);
        }
    }
    // The config checks of `new` hold for `new_with` too.
    let bad = IsolateConfig {
        vm_stack_slots: 1,
        ..IsolateConfig::default()
    };
    assert!(matches!(
        Isolate::new_with(bad, ExecOptions::jitless()),
        Err(IsolateError::Config(_))
    ));
}

#[test]
fn build_info_names_the_features_and_the_default_preset() {
    let b = crate::build_info();
    let mut want = Vec::new();
    if cfg!(feature = "jit") {
        want.push("jit");
    }
    if cfg!(feature = "become") {
        want.push("become");
    }
    if cfg!(feature = "spike-native") {
        want.push("spike-native");
    }
    if cfg!(feature = "ic-stats") {
        want.push("ic-stats");
    }
    assert_eq!(b.features, want.as_slice());
    assert_eq!(b.preset, ExecOptions::default().preset().name());
    let line = b.to_string();
    assert!(
        line.starts_with("features ") && line.contains("; preset "),
        "{line}"
    );
    let none = BuildInfo {
        features: &[],
        preset: "jitless",
    };
    assert_eq!(none.to_string(), "features none; preset jitless");
    let two = BuildInfo {
        features: &["jit", "become"],
        preset: "jit",
    };
    assert_eq!(two.to_string(), "features jit,become; preset jit");
}
