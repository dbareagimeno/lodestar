# `lodestar check` in CI

`lodestar check` is a gate: it audits the Markdown in your working tree and tells you, with a stable
exit code, whether it is still interpretable. It reads only — no file is ever modified — so it is
safe to run on any branch, in any job, as often as you like.

Every command and output on this page comes from a real run — against
[`examples/demo/`](../../examples/demo/README.md), or, in the last section, against the two-file
workspace built in [quickstart.md](quickstart.md#4-run-it-on-your-own-files).

- [What it judges](#what-it-judges)
- [Exit codes](#exit-codes)
- [`--json`](#--json)
- [`--sarif`](#--sarif)
- [A complete GitHub Actions workflow](#a-complete-github-actions-workflow)
- [Tuning the gate](#tuning-the-gate)

## What it judges

The **working tree**, always: the files as they are on disk under the workspace root, not a commit,
an index or a diff. Lodestar has no VCS integration — your CI already has the right files checked
out, and that is what gets audited.

```console
$ cd examples/demo
$ lodestar check
  ✗ [LINK-TARGET-MISSING] runbooks/incident-response.md: El enlace apunta a un documento que no existe: «runbooks/escalation.md».

10 documentos · 1 con errores · 0 avisos · NO VÁLIDO
$ echo $?
1
```

Use `--path` to audit a subdirectory without changing the job's working directory:

```bash
lodestar --path docs check
```

Diagnostic messages are currently in Spanish; the diagnostic codes, the JSON and SARIF field names
and the exit codes are English and stable. Those are the parts a pipeline should depend on.

## Exit codes

Frozen. A change to this table would be a breaking change:

| Code | Meaning | Typical CI reaction |
|---|---|---|
| `0` | Valid: no error diagnostics | Pass |
| `1` | Blocked: at least one `err` (or warnings promoted to blocking) | Fail the job |
| `2` | Invalid usage: unknown flag or subcommand | Fix the workflow |
| `3` | Runtime or I/O error, e.g. an invalid `.lodestar/config.yaml` | Fail loudly — it is not a verdict |

`3` deserves care: it is *not* "the workspace is fine". Treating any non-zero code as "found
problems" hides configuration errors, so a workflow that reports findings should distinguish `1`
from `3`.

## `--json`

`--json` prints the whole analysis: the document inventory, the per-document diagnostics, the link
graph and the verdict.

```console
$ lodestar check --json | jq 'keys'
[
  "dangling",
  "diagnostics",
  "documents",
  "incoming",
  "isolated",
  "outgoing",
  "recoveryPending",
  "valid"
]
```

The fields a pipeline usually wants:

| Field | What it holds |
|---|---|
| `valid` | The verdict: `true` when no diagnostic has level `err` |
| `diagnostics` | Map of document path → list of diagnostics (`level`, `code`, `msg`, `range`, `related`) |
| `documents` | Every document that was walked |
| `dangling` | Links whose target does not exist, with the source document and the link itself |
| `isolated` | Documents with no internal links in or out |
| `recoveryPending` | `true` if a publication was interrupted; the diagnostics then describe a recoverable intermediate state |

Some recipes, all run against the demo:

```console
$ lodestar check --json | jq '.valid'
false

$ lodestar check --json | jq -r '.diagnostics | to_entries[] | .key as $p | .value[] | "\(.level)\t\($p)\t\(.code)"'
err	runbooks/incident-response.md	LINK-TARGET-MISSING

$ lodestar check --json | jq '[.diagnostics[][] | select(.level == "err")] | length'
1

$ lodestar check --json | jq -c '.isolated'
["notes/scratchpad.md"]
```

Note that `check --json` still exits `1` when the workspace is blocked, so in a shell with `set -e`
you need `|| true` or an explicit capture of `$?` before you can post-process the output.

## `--sarif`

`--sarif` emits SARIF 2.1.0, the format GitHub code scanning, Azure DevOps and several editors
consume. Each error and warning becomes a result with a `ruleId` (the diagnostic code), a `level`
(`error` / `warning`) and the document it belongs to.

```console
$ lodestar check --sarif > lodestar.sarif
$ jq -e '.version == "2.1.0" and (.runs[0].results | length) > 0' lodestar.sarif
true
$ jq -r '.runs[0].results[] | "\(.level)\t\(.ruleId)\t\(.locations[0].physicalLocation.artifactLocation.uri)"' lodestar.sarif
error	LINK-TARGET-MISSING	runbooks/incident-response.md
```

The full document for the demo is small enough to read whole:

```console
$ lodestar check --sarif
{
  "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
  "runs": [
    {
      "results": [
        {
          "level": "error",
          "locations": [
            {
              "physicalLocation": {
                "artifactLocation": {
                  "uri": "runbooks/incident-response.md"
                }
              }
            }
          ],
          "message": {
            "text": "El enlace apunta a un documento que no existe: «runbooks/escalation.md»."
          },
          "ruleId": "LINK-TARGET-MISSING"
        }
      ],
      "tool": {
        "driver": {
          "informationUri": "https://github.com/dbareagimeno/lodestar",
          "name": "lodestar",
          "rules": []
        }
      }
    }
  ],
  "version": "2.1.0"
}
```

Locations are document-level: the result names the file, not a line range. Alerts therefore attach
to the file rather than to a specific line.

**A few diagnostics have no location at all.** `WORKSPACE-EMPTY` and `PATH-NOT-UTF8` are about the
workspace, not about a document you wrote — there is no file to point at — so their results are
emitted **without a `locations` array**, which is what SARIF 2.1.0 prescribes for a finding that
belongs to no artifact. They still carry their `ruleId`, `level` and `message`, and they still count
toward the exit code. If you post-process the SARIF, do not assume `.locations[0]` exists: the `jq`
one-liner above would print `null` for those rows. Use
`.locations[0].physicalLocation.artifactLocation.uri // "(workspace)"` if you want a placeholder.

`--json` and `--sarif` are mutually exclusive; pick one per invocation.

## A complete GitHub Actions workflow

This workflow downloads the released Linux binary, verifies its checksum, audits the repository, and
publishes the findings to code scanning — while still failing the job when the gate blocks.

```yaml
name: Docs gate

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read
  security-events: write   # required to upload SARIF to code scanning

jobs:
  lodestar:
    runs-on: ubuntu-latest
    env:
      LODESTAR_VERSION: v0.6.2
      LODESTAR_TARGET: x86_64-unknown-linux-gnu
    steps:
      - uses: actions/checkout@v4

      - name: Install lodestar
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          set -euo pipefail
          gh release download "$LODESTAR_VERSION" \
            --repo dbareagimeno/lodestar \
            --pattern "lodestar-cli-$LODESTAR_VERSION-$LODESTAR_TARGET.tar.gz" \
            --pattern "SHA256SUMS-$LODESTAR_TARGET.txt"
          # Releases ship a checksums file; verify it when present.
          if [ -f "SHA256SUMS-$LODESTAR_TARGET.txt" ]; then
            sha256sum -c "SHA256SUMS-$LODESTAR_TARGET.txt"
          fi
          tar -xzf "lodestar-cli-$LODESTAR_VERSION-$LODESTAR_TARGET.tar.gz"
          mkdir -p "$HOME/.local/bin"
          mv lodestar lodestar-mcp "$HOME/.local/bin/"
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"

      - name: Run lodestar check
        id: check
        run: |
          set +e
          lodestar check --sarif > lodestar.sarif
          echo "exit_code=$?" >> "$GITHUB_OUTPUT"

      - name: Upload SARIF to code scanning
        if: always()
        uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: lodestar.sarif
          category: lodestar

      - name: Report the human-readable diagnostics
        if: steps.check.outputs.exit_code != '0'
        run: lodestar check || true

      - name: Fail on a runtime error
        if: steps.check.outputs.exit_code == '2' || steps.check.outputs.exit_code == '3'
        run: |
          echo "::error::lodestar could not produce a verdict (exit ${{ steps.check.outputs.exit_code }})"
          exit 1

      - name: Fail when the gate blocks
        if: steps.check.outputs.exit_code == '1'
        run: |
          echo "::error::lodestar check found blocking diagnostics"
          exit 1
```

Why it is shaped like this:

- **Pin the version.** `LODESTAR_VERSION` is an explicit tag, not `latest`: a gate whose engine
  changes without you noticing is not a gate.
- **`set +e` around `check`.** The command exits `1` on findings; without capturing `$?` the step
  would fail before the SARIF is uploaded, and you would lose the report exactly when you need it.
- **`if: always()` on the upload.** Same reason, from the other side.
- **Separate `1` from `2`/`3`.** Both fail the build, but only `1` means "we looked and found
  problems". `2` and `3` mean the gate did not run.
- **`security-events: write`.** Without it, `upload-sarif` fails. On pull requests from forks GitHub
  withholds that permission; if you need fork coverage, drop the SARIF upload for those runs and
  rely on the plain `lodestar check` output.

If you prefer not to use the GitHub CLI, replace the download with `curl`:

```bash
curl -sSfLO "https://github.com/dbareagimeno/lodestar/releases/download/$LODESTAR_VERSION/lodestar-cli-$LODESTAR_VERSION-$LODESTAR_TARGET.tar.gz"
```

Building from source works too (`cargo install --git https://github.com/dbareagimeno/lodestar
lodestar-cli`), at the cost of a compile per run unless you cache it.

## Tuning the gate

By default only errors block. If you want warnings to block as well, add
`.lodestar/config.yaml` to the repository:

```yaml
gate:
  blockWarnings: true
```

The effect, on the two-file workspace from
[quickstart.md](quickstart.md#4-run-it-on-your-own-files) — whose only finding is a warning:

```console
$ lodestar check
  ! [LINK-TARGET-MISSING] onboarding.md: El enlace apunta a un fichero del proyecto que no existe: «office-map.png».

2 documentos · 0 con errores · 1 avisos · NO VÁLIDO (avisos bloqueados por .lodestar/config.yaml)
$ echo $?
1
```

Without that file, the same workspace passes:

```console
$ lodestar check
  ! [LINK-TARGET-MISSING] onboarding.md: El enlace apunta a un fichero del proyecto que no existe: «office-map.png».

2 documentos · 0 con errores · 1 avisos · VÁLIDO
$ echo $?
0
```

A `config.yaml` the engine cannot use is exit `3`, never a silent fallback to defaults: a typo must
not quietly loosen a gate you tightened on purpose. That covers malformed YAML, a key the engine
does not recognise, and a file that exists but cannot be read. A file that is simply *absent* is not
an error — running without any configuration is a supported, permanent state.

### Diagnostic families

The same file can lower or raise the severity of individual diagnostic families, and restrict which
directories are walked. Configuration can only ever *restrict*: its absence never stops Lodestar
from working, and no key can grant a permission the engine does not give by default.

The keys under `validation` are **families**, not diagnostic codes. There are exactly five:

| Family | Default | What it covers |
|---|---|---|
| `malformedFrontmatter` | `error` | Frontmatter that cannot be parsed: `FM-UNCLOSED`, `FM-YAML-INVALID` |
| `danglingDocumentLinks` | `error` | A link to a Markdown **document** that does not exist (`LINK-TARGET-MISSING` whose missing target is a `.md`) |
| `missingWorkspaceFiles` | `warning` | A link to a **project file** that does not exist (`LINK-TARGET-MISSING` whose missing target is not a document) — an image, a script, a PDF |
| `caseMismatch` | `warning` | Capitalisation that does not survive a case-sensitive filesystem (`LINK-CASE-MISMATCH`) |
| `isolatedDocuments` | `ignore` | Documents with no internal links in or out. Currently produces no diagnostic — isolation is a queryable property, not a finding — so this key is accepted but has no effect |

Each key takes `error`, `warning` or `ignore`. An override reclassifies *every* diagnostic in the
family; `ignore` suppresses them:

```console
$ cat .lodestar/config.yaml
validation:
  missingWorkspaceFiles: ignore
$ lodestar check

2 documentos · 0 con errores · 0 avisos · VÁLIDO
$ echo $?
0
```

Note that `LINK-TARGET-MISSING` — one code — is split across two families by the kind of target
that is missing. That is deliberate: a broken link between two documents breaks the knowledge graph,
while a missing image usually does not, so they are worth different severities out of the box.

Writing a **code** where a family belongs is an error, not a silent no-op:

```console
$ cat .lodestar/config.yaml
validation:
  "LINK-TARGET-MISSING": ignore
$ lodestar check
error: error de IO: .lodestar/config.yaml inválido: «LINK-TARGET-MISSING» no es una familia de validación de `§20.9`. Las claves de `validation` son FAMILIAS, no códigos de diagnóstico. Familias válidas: malformedFrontmatter, danglingDocumentLinks, missingWorkspaceFiles, caseMismatch, isolatedDocuments
$ echo $?
3
```

Accepting that key would be worse than rejecting it: you would believe the diagnostic was silenced
and keep seeing it on every run.

---

See also: [quickstart.md](quickstart.md) for installing and reading the output, and
[mcp-clients.md](mcp-clients.md) for the agent-facing half of the same engine.
