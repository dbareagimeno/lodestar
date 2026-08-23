# Lodestar

**Make any Markdown repository navigable, queryable and safe for AI agents.**

[![CI](https://github.com/dbareagimeno/lodestar/actions/workflows/ci.yml/badge.svg)](https://github.com/dbareagimeno/lodestar/actions/workflows/ci.yml)
[![Rust 1.80+](https://img.shields.io/badge/Rust-1.80%2B-dea584?logo=rust)](rust-toolchain.toml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Lodestar is a local, transactional engine that lets agents discover, query, understand and modify a
network of Markdown documents without converting it to a proprietary format. It reads the structure
that already exists, interprets the frontmatter you already use, resolves its links, and protects
changes with validation, concurrency control and rollback.

```bash
cd my-project
lodestar-mcp
```

That is all. You do not need to initialize the repository, create a special index or adapt your
documents. Lodestar discovers `.md` files recursively, honours `.gitignore` and `.lodestarignore`,
and uses the current directory as the workspace.

> Your Markdown remains the single source of truth: human-readable, versionable with whatever tools
> you prefer, and usable without Lodestar.

## What it gives you

- **Precise context for agents.** Search first and then retrieve only the document, section or
  fields you need; it does not dump the whole repository into the context window.
- **Queries over your own metadata.** Filter any YAML property with typed operators, dot notation
  and conditions over the graph, without imposing a schema.
- **A real view of the relationships.** It resolves Markdown links, backlinks, isolated documents,
  broken links, paths, cycles and components at any depth.
- **Analysis before changing anything.** It computes the blast radius and the affected references
  before you move or delete a document.
- **Recoverable changes.** Plan and validate in memory, apply through an atomic transaction, and
  keep a receipt you can revert from.
- **A quality gate for CI.** It audits the working tree and produces human, JSON or SARIF output
  with stable exit codes.
- **Local-first and file-first.** The server speaks over `stdio`; the SQLite/FTS5 cache is derived
  and can be rebuilt from the Markdown.

## How an agent works with Lodestar

```text
orient → search → read → inspect metadata and relationships
    → analyze impact → plan → apply → validate → revert if needed
```

Lodestar exposes that journey over MCP:

| Need | Tools |
|---|---|
| Understand the workspace | `workspace_status`, `knowledge_search`, `knowledge_get`, `metadata_inspect` |
| Analyze the knowledge | `graph_query`, `impact_analyze`, `knowledge_check` |
| Change it safely | `change_plan`, `change_apply`, `change_revert` |

The `readonly` profile offers the seven read and verification tools. The `standard` profile, used by
default, adds planning, applying and reverting changes.

## Quickstart

### 1. Install the binaries

Every [GitHub release](https://github.com/dbareagimeno/lodestar/releases/latest) ships prebuilt
binaries for macOS (Apple Silicon), Linux (x86_64) and Windows (x86_64). Download the archive for
your platform, unpack it and put both executables on your `PATH`:

```bash
tar -xzf lodestar-cli-v*-aarch64-apple-darwin.tar.gz   # .zip on Windows
mv lodestar lodestar-mcp ~/.local/bin/                 # or any directory on your PATH
```

Releases published after `v0.5.0` also ship a `SHA256SUMS-<target>.txt` next to each archive: verify
it with `shasum -a 256 -c SHA256SUMS-<target>.txt` (or `sha256sum -c`) before unpacking.

The binaries are unsigned, so macOS and Windows may ask for confirmation the first time.

You can also build from source. The CLI requires Rust 1.80 or later; the MCP server requires Rust
1.88 or later:

```bash
cargo install --git https://github.com/dbareagimeno/lodestar lodestar-cli
cargo install --git https://github.com/dbareagimeno/lodestar lodestar-mcp
```

Either way you get:

- `lodestar`, the CLI for validation and maintenance;
- `lodestar-mcp`, the local server that connects the workspace to an agent.

No Node.js, no git and no GUI libraries required.

### 2. Try it on the demo workspace

The repository ships [`examples/demo/`](examples/demo/README.md): ten Markdown documents about a
fictional service, with two deliberate defects — one broken link and one orphan document.

```console
$ git clone https://github.com/dbareagimeno/lodestar
$ cd lodestar/examples/demo
$ lodestar check
  ✗ [LINK-TARGET-MISSING] runbooks/incident-response.md: El enlace apunta a un documento que no existe: «runbooks/escalation.md».

10 documentos · 1 con errores · 0 avisos · NO VÁLIDO
$ echo $?
1
```

Ten documents were walked, the broken link was found, and the non-zero exit code is what makes
`lodestar check` usable as a CI gate. Nothing on disk was modified. (Diagnostic messages are
currently emitted in Spanish; the diagnostic codes such as `LINK-TARGET-MISSING` and the exit codes
are the stable part of the contract.)

The same command works on any project of yours:

```bash
cd /path/to/my-project
lodestar check
lodestar check --json
lodestar check --sarif > lodestar.sarif
```

### 3. Connect your MCP client

Configure the client to launch `lodestar-mcp` over `stdio`. In [Claude
Code](https://claude.com/claude-code) that is one line:

```bash
claude mcp add lodestar -- lodestar-mcp --root /absolute/path/to/project
```

In clients configured with JSON, the equivalent definition is:

```json
{
  "mcpServers": {
    "lodestar": {
      "command": "lodestar-mcp",
      "args": [
        "--root",
        "/absolute/path/to/project",
        "--profile",
        "readonly"
      ]
    }
  }
}
```

You can omit `--root` if the client starts the process inside the project. Use `readonly` to explore
or review; switch to `standard` when you want to allow transactional changes.

A full session against the demo — typed query, orphan detection, impact analysis, and a
plan/apply/revert round trip, with the real request and response of every tool call — is written out
in [`examples/demo/README.md`](examples/demo/README.md).

## Works with the Markdown you already have

There are no mandatory fields and no reserved file names. A document can be plain Markdown:

```markdown
# Credential rotation

See also the [deployment runbook](../runbooks/deploy.md).
```

Or it can use whatever YAML frontmatter makes sense for your team:

```markdown
---
status: accepted
priority: 2
owners:
  - platform
service:
  tier: critical
---

# Credential rotation

See also the [deployment runbook](../runbooks/deploy.md).
```

Lodestar preserves the real YAML types and allows queries such as:

```text
status = "accepted" and priority >= 2
owners contains "platform"
service.tier = "critical"
graph.backlinks = 0
```

`metadata_inspect` lets an agent first discover which fields exist, their types, their coverage and
their frequent values. That way it can understand the conventions of an unfamiliar project without
you having to maintain a parallel schema.

## Graph and impact

Every internal Markdown link forms an edge of the graph. Lodestar recognizes inline links, reference
links, anchors, external targets and relative paths between documents sitting at any depth.

`graph_query` lets you ask for:

- backlinks and outgoing links;
- incoming, outgoing or bidirectional neighborhoods;
- isolated documents and links without a target;
- the path between two documents;
- cycles and components of the graph.

Before a `move` or a `delete`, `impact_analyze` identifies directly and transitively affected
documents and computes the risk level without touching disk.

## Safe, recoverable changes

The `standard` profile deliberately separates thinking from writing:

1. `change_plan` normalizes the operations, simulates the result in memory, computes the semantic
   diff, evaluates the impact and validates whether the change can be applied.
2. `change_apply` checks that the workspace has not changed since the plan, and publishes through
   staging, a lock, recovery copies, a write-ahead journal and atomic renames.
3. `knowledge_check` confirms the resulting state.
4. `change_revert` restores a recent transaction from its receipt if you need to undo it.

The available operations cover creation, surgical frontmatter edits, body or text replacement,
section editing, moves and deletions. Compatible operations can also be applied over selections
obtained through a query.

Deterministic document and workspace revisions provide optimistic concurrency control: if a person
or another tool changes a file between the plan and the apply, Lodestar rejects the stale write.

## CLI

The CLI is a small facade for people, scripts and CI:

| Command | Use |
|---|---|
| `lodestar check` | Audits the working tree |
| `lodestar reindex` | Rebuilds the derived `.lodestar/index.db` cache from the Markdown |
| `lodestar migrate-from-okf --dry-run` | Diagnoses legacy OKF conventions without modifying files |

To operate on another directory without changing the `cwd`:

```bash
lodestar --path /path/to/project check
```

The exit codes of `check` are stable: `0` no errors, `1` validation blocked, `2` invalid usage and
`3` runtime or I/O error.

## Migrating from OKF

Repositories created with the old OKF format are still valid Markdown and can be opened directly.
The migration command is diagnostic only:

```bash
lodestar --path /path/to/project migrate-from-okf --dry-run
```

It reports on legacy indexes, `okf_version` and tag indexes, but it never modifies the project. See
the [changelog](CHANGELOG.md) for how the format evolved and what is incompatible between releases.

## Architecture

```text
                        ┌──────────────────────────┐
Markdown repository ───►│ discovery + parser       │
 source of truth        │ metadata · links · query ├──► MCP / agents
                        │ graph · impact · diff    ├──► CLI / CI
                        │ transactions · recovery  │
                        └────────────┬─────────────┘
                                     ▼
                              SQLite / FTS5
                             rebuildable cache
```

The domain logic is shared by both facades:

```text
crates/
  lodestar-core/        document model, metadata, links, query, graph and diff
  lodestar-store/       SQLite/FTS5 cache and watcher
  lodestar-workspace/   discovery, I/O and recoverable publication
  lodestar-app/         use cases shared by the CLI and MCP
  lodestar-cli/         facade for people and CI
  lodestar-mcp/         MCP facade over stdio for agents
  lodestar-fixtures/    shared test workspaces
```

The core performs no I/O and the facades do not reimplement the semantics. A query or a validation
produces the same result regardless of the consumer.

## Development

```bash
cargo test --workspace --locked
cargo test -p lodestar-workspace --features test-failpoints --locked
cargo test -p lodestar-app --features test-failpoints --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

CI runs formatting, strict linting, build, documentation and tests — including the crash-recovery
scenarios — on Linux, macOS and Windows.

The internal E33 performance bank is `lodestar-bench` (`publish = false`). Its
[`README`](crates/lodestar-bench/README.md) links the usage guide and the current-metrics reference;
the release gate is documented in [`docs/qa/testbench/README.md`](docs/qa/testbench/README.md). It
measures the seven read tools and cold-open, and its ratified absolute ceilings apply only to the
`disk-reparseo` 10k variant on the explicitly identified release machine. Shared CI runs only the
cheap smoke; SQLite measurements remain evidence and are neither an optimization promise nor a
veto.

## Documentation

Start with the [guided demo](examples/demo/README.md): every command and every MCP call in it comes
from a real run.

The **user guide** lives in [`docs/user/`](docs/user/) and is written in English:

| Document | Contents |
|---|---|
| [Quickstart](docs/user/quickstart.md) | Install, run the first `check`, read the output, exit codes |
| [MCP clients](docs/user/mcp-clients.md) | Per-client configuration, `--root`, profiles, a tour of the ten tools |
| [CI](docs/user/ci.md) | `check` as a gate: exit codes, `--json`, `--sarif`, a complete GitHub Actions workflow |
| [Query language](docs/user/query-language.md) | `where` and `filter`: types, operators, namespaces, and the declared limits |
| [Safe changes](docs/user/safe-changes.md) | `change_plan` → `change_apply` → `change_revert`: concurrency, receipts, crash recovery |

The documents below govern the development of the repository. They are **written in Spanish by
design** (`ARCHITECTURE.md §21.1`: the public surface is in English, the internal material that
governs development stays in Spanish):

| Document | Contents |
|---|---|
| [Architecture](ARCHITECTURE.md) | Current design and engine invariants |
| [MCP contract](contracts/mcp.yml) | Surface and semantics of the tools |
| [Implementation status](IMPLEMENTATION_STATUS.md) | Verified capabilities and traceability |
| [Decisions](decisiones/README.md) | Open and ratified product decisions |
| [Changelog](CHANGELOG.md) | Per-release history of changes |
| [Releasing](RELEASING.md) | Publication process |

## Roadmap

There is no separate roadmap document to keep in sync: the living record of what is decided, what is
still open and why is [`decisiones/`](decisiones/README.md) (in Spanish, like the rest of the internal
material). Each entry states the question, the options considered and the ratified answer, so what
comes next is readable from the open ones. Anything not implemented today is not promised here.

## License

Lodestar is distributed under **MIT OR Apache-2.0**, at your option. See [LICENSE-MIT](LICENSE-MIT)
and [LICENSE-APACHE](LICENSE-APACHE).
