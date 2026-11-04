**What and why**

**How it was checked** (tests, snapshot review, smoke test, headset)

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] UI changed: snapshots reviewed and committed
- [ ] New dependency: reason given, `cargo deny check` passes
- [ ] CHANGELOG.md updated under Unreleased
