<!-- Thanks! Two ground rules from CONTRIBUTING.md:
     - Bug fixes and documentation improvements: PR directly, fill the checklist.
     - Features: need an accepted issue first — link it below. -->

## What this changes

<!-- One or two sentences. If it fixes an issue, write "Fixes #NN". -->

## For features: prior issue

<!-- Required for feature PRs (issues-first policy): link the accepted issue.
     Delete this section for bug/docs PRs. -->

## Checklist (same gates CI runs)

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --workspace --locked`
- [ ] `cargo test -p lodestar-workspace --features test-failpoints --locked` and
      `cargo test -p lodestar-app --features test-failpoints --locked`
      (crash-recovery / publication-window tests — **not** covered by the workspace run)
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`
- [ ] If `examples/demo/` or the engine behaviour it shows changed: `scripts/demo-smoke.sh` passes
- [ ] If the MCP boundary changed (`contracts/mcp.yml`, tool schemas, `core::types` wire types):
      the contract file and the code agree — say so explicitly in the description
- [ ] Public-facing docs (English) updated if the surface changed; internal docs stay in Spanish
