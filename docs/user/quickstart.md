# Quickstart

Install Lodestar, run your first check, and learn to read what it tells you. Ten minutes, no
project setup: Lodestar works on the Markdown you already have — no `init`, no config file, no
mandatory frontmatter.

Every command and every output on this page comes from a real run.

- [1. Install](#1-install)
- [2. Run your first check](#2-run-your-first-check)
- [3. Read the output](#3-read-the-output)
- [4. Run it on your own files](#4-run-it-on-your-own-files)
- [5. Connect an agent](#5-connect-an-agent)
- [What to read next](#what-to-read-next)

## 1. Install

You get two executables either way:

- `lodestar` — the command-line interface for validation and maintenance;
- `lodestar-mcp` — the local MCP server that connects a workspace to an agent.

No Node.js, no git, no GUI libraries are required.

### Option A — prebuilt binaries (recommended)

Every [release](https://github.com/dbareagimeno/lodestar/releases/latest) ships one archive per
platform, named `lodestar-cli-<version>-<target>.<ext>`:

| Platform | Asset |
|---|---|
| macOS (Apple Silicon) | `lodestar-cli-vX.Y.Z-aarch64-apple-darwin.tar.gz` |
| Linux (x86_64) | `lodestar-cli-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |
| Windows (x86_64) | `lodestar-cli-vX.Y.Z-x86_64-pc-windows-msvc.zip` |

With the [GitHub CLI](https://cli.github.com/):

```console
$ gh release download v0.6.0 --repo dbareagimeno/lodestar \
    --pattern 'lodestar-cli-*-aarch64-apple-darwin.tar.gz'
$ tar -xzf lodestar-cli-v0.6.0-aarch64-apple-darwin.tar.gz
$ ./lodestar --version
lodestar 0.6.0
```

Or with `curl`, straight from the release URL:

```console
$ curl -sSfLO https://github.com/dbareagimeno/lodestar/releases/download/v0.6.0/lodestar-cli-v0.6.0-x86_64-unknown-linux-gnu.tar.gz
$ tar -xzf lodestar-cli-v0.6.0-x86_64-unknown-linux-gnu.tar.gz
```

Both archives contain exactly two files, `lodestar` and `lodestar-mcp` (`.exe` on Windows). Put
them anywhere on your `PATH`:

```bash
mv lodestar lodestar-mcp ~/.local/bin/
```

**Verify the download.** Releases ship a `SHA256SUMS-<target>.txt`
next to each archive. Download it into the same directory and check the archive before unpacking:

```console
$ shasum -a 256 -c SHA256SUMS-aarch64-apple-darwin.txt
lodestar-cli-v0.6.0-aarch64-apple-darwin.tar.gz: OK
```

On Linux use `sha256sum -c` instead. A non-zero exit code means the file does not match what the
release pipeline built — do not run it.

The binaries are not signed or notarized, so macOS and Windows may ask for confirmation the first
time you run them.

### Option B — build from source

Requires Rust 1.80 or later:

```bash
cargo install --git https://github.com/dbareagimeno/lodestar lodestar-cli
cargo install --git https://github.com/dbareagimeno/lodestar lodestar-mcp
```

This installs the same two binaries into `~/.cargo/bin`.

## 2. Run your first check

The repository ships a small demo workspace: ten Markdown documents about a fictional telemetry
service, with two deliberate defects — one broken link and one orphan document.

```console
$ git clone https://github.com/dbareagimeno/lodestar
$ cd lodestar/examples/demo
$ lodestar check
  ✗ [LINK-TARGET-MISSING] runbooks/incident-response.md: El enlace apunta a un documento que no existe: «runbooks/escalation.md».

10 documentos · 1 con errores · 0 avisos · NO VÁLIDO
$ echo $?
1
```

Ten documents were walked, the broken link was found, and the exit code is `1`. Nothing on disk was
modified: `check` only reads.

> **A note on language.** Diagnostic messages and the summary line are currently emitted in Spanish.
> The stable, machine-readable parts of the contract are in English and are not going to move under
> you: the diagnostic codes (`LINK-TARGET-MISSING`, …), the JSON and SARIF field names, and the exit
> codes.

## 3. Read the output

### The diagnostic line

```text
  ✗ [LINK-TARGET-MISSING] runbooks/incident-response.md: El enlace apunta a un documento que no existe: «runbooks/escalation.md».
```

Four parts, in order:

1. the **severity marker** — `✗` for an error, `!` for a warning;
2. the **diagnostic code** in brackets — stable and in English; this is what you filter, group and
   suppress on;
3. the **document**, as a path relative to the workspace root;
4. the **message**, after the colon.

Two severities reach the output:

| Marker | Severity | Meaning | Effect on the exit code |
|---|---|---|---|
| `✗` | `err` | The workspace cannot be interpreted as its author intended — a Markdown link points at a document that does not exist, frontmatter that does not parse, a link that escapes the root. | Blocks: exit `1` |
| `!` | `warn` | Something is suspicious but not broken — a link to a non-Markdown project file that is missing, a path that only matches ignoring case. | Does not block by default |

### The summary line

```text
10 documentos · 1 con errores · 0 avisos · NO VÁLIDO
```

Documents walked · error diagnostics · warning diagnostics · verdict (`VÁLIDO` / `NO VÁLIDO`). The
verdict is the same one an agent gets from the `knowledge_check` MCP tool: one engine, one truth.

### Exit codes

They are frozen — scripts and CI can rely on them:

| Code | Meaning |
|---|---|
| `0` | No error diagnostics: the workspace is valid |
| `1` | Validation blocked (at least one `err`, or warnings promoted to blocking) |
| `2` | Invalid usage (unknown flag or subcommand) |
| `3` | Runtime or I/O error (for example an unreadable or invalid `.lodestar/config.yaml`) |

Both failure modes, for real (the second one from a scratch directory — delete the `.lodestar/`
you create here afterwards):

```console
$ lodestar check --nope
error: unexpected argument '--nope' found

Usage: lodestar check [OPTIONS]

For more information, try '--help'.
$ echo $?
2
```

```console
$ mkdir -p .lodestar
$ printf 'gate:\n  blockWarnings: "yes please"\n' > .lodestar/config.yaml
$ lodestar check
error: error de IO: .lodestar/config.yaml inválido: gate.blockWarnings: invalid type: string "yes please", expected a boolean at line 2 column 18
$ echo $?
3
```

A malformed config is an explicit error, never a silent fallback to defaults — falling back would
quietly relax a gate you asked to tighten.

## 4. Run it on your own files

Any directory is a workspace. To see both severities on something you control, build this two-file
workspace:

```console
$ mkdir -p lodestar-tour && cd lodestar-tour
$ cat > index.md <<'EOF'
# Handbook

- [Onboarding](onboarding.md)
EOF
$ cat > onboarding.md <<'EOF'
---
status: draft
owners: [platform]
---

# Onboarding

Back to the [handbook](index.md). See the [office map](office-map.png).
EOF
$ lodestar check
  ! [LINK-TARGET-MISSING] onboarding.md: El enlace apunta a un fichero del proyecto que no existe: «office-map.png».

2 documentos · 0 con errores · 1 avisos · VÁLIDO
$ echo $?
0
```

The missing image is reported as a warning and does not block; the two documents link to each other
correctly, so nothing is an error. The frontmatter is entirely yours: `status` and `owners` mean
whatever your team decides, and Lodestar keeps their real YAML types for querying.

On a real project:

```bash
cd /path/to/my-project
lodestar check
```

Lodestar walks `.md` files recursively from the current directory, honouring `.gitignore` and
`.lodestarignore`. To audit another directory without changing your shell's working directory:

```bash
lodestar --path /path/to/my-project check
```

`--path` never climbs to ancestors: the directory you name is the workspace root, and every path in
the output is relative to it.

## 5. Connect an agent

The CLI is the smaller half. The other half is `lodestar-mcp`, which exposes the same engine to an
agent over `stdio`:

```bash
claude mcp add lodestar -- lodestar-mcp --root /absolute/path/to/project
```

Per-client configuration, the two profiles and a tour of the ten tools are in
[mcp-clients.md](mcp-clients.md).

## What to read next

| If you want to… | Read |
|---|---|
| Configure your MCP client and understand the ten tools | [mcp-clients.md](mcp-clients.md) |
| Make `lodestar check` a gate in CI, with SARIF in code scanning | [ci.md](ci.md) |
| See a full agent session — typed query, impact analysis, plan/apply/revert | [`examples/demo/README.md`](../../examples/demo/README.md) |
| Know exactly what each tool accepts and returns | [`contracts/mcp.yml`](../../contracts/mcp.yml) (in Spanish, like the rest of the internal material) |
| Understand what the project is and where it is going | [`README.md`](../../README.md) |
