# ep9

ep9 is a JavaScript and TypeScript engine in Rust. One engine runs in three modes: JIT, AOT-jitless and hybrid. The parser is oxc. tsrs runs only as an external type checker and is never in the engine graph.

Status: milestone M0 (contracts, spikes, skeleton). The plan is in [docs/PLAN.md](docs/PLAN.md); the details are in [docs/plan/](docs/plan/).

## Build

Rust 1.99.1 comes from `rust-toolchain.toml`; rustup installs it on first use.

```sh
bash scripts/sync-test262.sh     # third_party/test262 at third_party/test262.commit
bash scripts/sync-tsrs.sh        # third_party/tsrs at third_party/tsrs.commit, builds the `tsrs` binary
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The sync scripts work from any directory. A second run with the same pin and patches does not fetch again. `cargo build --profile dev-opt` gives optimized builds without fat LTO, for fast local work.

## License

ep9 is licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Code copied into this repository from other projects keeps its own licence; [THIRD-PARTY-LICENSE](THIRD-PARTY-LICENSE) lists it. The licence rules are in [12-governance §1](docs/plan/12-governance.md#1-licence).

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in ep9 by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
