# calprobe

The re-run kit of the INTL I1 probe (decision INTL-I1-D2; the H36 fp-check entry 2 reason). It
counts, at run time, every call of `core_maths` (the float methods that reach `libm`) and of the
astronomy code of `calendrical_calculations` (`astronomy.rs`, `chinese_based.rs`) on the calendar
paths that ep9 reaches through temporal_rs and icu_calendar.

```text
tools/calprobe/run.sh
  1 lockcheck.py   every version in tools/calprobe/Cargo.lock is the root Cargo.lock version
  2 instr.py       counting copies of core_maths and calendrical_calculations (registry source
                   -> target/work; a counter at the start of every non-const fn)
  3 cargo build    the probe with the copies patched in (a copy of this crate), release profile
                   of ep9 (fat LTO, one codegen unit, panic abort)
  4 run            icu      18 icu_calendar kinds x 24,580 ISO dates          0 and 0
                   temporal 17 ids x 297 years, every Temporal operation      0 and 0
                   control  China::gb_t_33661_2017 and its Korean form        all > 0
```

Exit 0 only if all hold. The control proves that the counters count: the opt-in astronomical
functions make thousands of `sin` calls per year. ep9 never calls them, and they import the host
`fmod` (through `core_maths::rem_euclid`), so `cargo xtask fp-check` fails if they are ever linked.

When to run: at every bump of icu_calendar, calendrical_calculations, core_maths or temporal_rs
(INTL-I1-D5, the BUMP gate), with the root Cargo.lock of the bump. First copy the new root lock
here (`cp Cargo.lock tools/calprobe/Cargo.lock`, then `cargo metadata --manifest-path
tools/calprobe/Cargo.toml` drops the packages the probe does not use) and update the pins in
`Cargo.toml`. The `TOTAL ... checksum=` values of two versions are equal when the results are.

This crate has its own `[workspace]`; the root `Cargo.toml` lists `tools/calprobe` in `exclude`.
It is a tool, never part of the engine graph.
