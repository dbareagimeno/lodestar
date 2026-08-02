# Contributing to Lodestar

Thanks for taking the time. Lodestar is a small, single-maintainer project with a deliberate
process, so this document is mostly about *where* a contribution should start — that is the part
that saves everyone the most work.

Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Issues first

**Bugs and documentation fixes: open a pull request directly.** No issue required. Include a
reproduction (the smallest workspace and the exact command or MCP call that shows the problem) and
go through the [PR checklist](#pull-request-checklist).

**Features: open an issue first.** A feature is anything that changes behavior — a new MCP tool or
tool argument, a new CLI flag, an extension of the query language, a change to a diagnostic code or
an exit code. In that issue the maintainer decides whether the feature goes through the repository's
design process before any code is written. A feature pull request that arrives without a prior
issue is not rejected on principle, but it may sit unread until that conversation has happened —
the discussion belongs upstream of the code, where changing your mind is still cheap.

**Security reports never go in an issue.** Use the private channel described in
[SECURITY.md](SECURITY.md).

GitHub Discussions is intentionally disabled for now. Issues are the channel; if the traffic ever
justifies a second one, it will be enabled then.

## Running the gates locally

These are the same commands CI runs. Run them before opening a pull request:

```bash
cargo test --workspace --locked
cargo test -p lodestar-workspace --features test-failpoints --locked
cargo test -p lodestar-app --features test-failpoints --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

`--features test-failpoints` is **not optional**. `cargo test --workspace` does not enable optional
features, so it neither runs nor counts those tests — and they are the ones covering crash recovery
and the publication window (the guarantee that an interrupted write never leaves a `.md` file half
written). Both crates need their own invocation: `lodestar-workspace` and `lodestar-app`.

You need a Rust toolchain (see [`rust-toolchain.toml`](rust-toolchain.toml)) and nothing else — no
Node.js, no git library, no GUI dependencies. CI additionally verifies that `lodestar-core` stays
pure (no `tokio`/`rusqlite`/`git2`/`notify`/`tauri` in its dependency tree); if you add a dependency
there, check it with `cargo tree -p lodestar-core`.

## Pull request checklist

- The six gate commands above pass locally.
- A bug fix comes with a test that fails without the fix.
- If the change touches the MCP surface, [`contracts/mcp.yml`](contracts/mcp.yml) is updated in the
  same pull request — the contract and the tools are not allowed to drift.
- Public items have `///` documentation.
- Documentation that becomes false is updated in the same pull request.
- For a feature: the pull request links the issue where it was agreed.
- The diff contains only what the pull request is about (no drive-by reformatting).

## What to expect from the process

The repository is developed spec-first: a change of behavior starts as a ratified story in
[`requirements/`](requirements/), the tests are written before the implementation by a different
author, and the result is reviewed by a blind reviewer that sees only the spec and the diff. That
process is described in [`docs/WORKFLOWS.md`](docs/WORKFLOWS.md) (in Spanish).

**You are not required to follow it.** It is the maintainer's process for integrating work, not a
bar you have to clear to open a pull request. What it does mean in practice: reviews are strict and
specific, a change of behavior will usually be asked to come with the spec written down first, and
"it works" is not by itself an argument — the invariants in [`CLAUDE.md`](CLAUDE.md) and the design
in [`ARCHITECTURE.md`](ARCHITECTURE.md) are.

## Language

Lodestar splits languages by **audience**, not by directory
([`ARCHITECTURE.md §21.1`](ARCHITECTURE.md)):

- **English** for everything an adopter reads before deciding: `README.md`, `docs/user/`,
  `examples/demo/`, this file, `SECURITY.md`, `CODE_OF_CONDUCT.md` and the `.github/` templates.
- **Spanish** for everything that governs the development of the repository: `ARCHITECTURE.md`,
  `DECISIONES.md`, `requirements/`, `docs/`, `contracts/`, source comments, error messages on the
  wire and commit messages.

So yes — once you go past the README you will find the internal documentation in Spanish, and that
is by design rather than an oversight: the maintainer is a Spanish speaker and a half-translated
corpus is worse than a consistently monolingual one.

For your contribution this means **code comments and commit messages in Spanish**, with technical
identifiers in English (types, functions, fields and diagnostic codes stay as they are). Issue and
pull request descriptions are fine in either language.

## License

By contributing you agree that your contribution is licensed under **MIT OR Apache-2.0**, the same
terms as the rest of the project ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
