# Lodestar demo workspace

A tiny, self-contained Markdown workspace — the docs of **Atlas**, a fictional
telemetry service — built to show what lodestar does in about two minutes.

It contains 10 documents under [overview](overview.md), [architecture](architecture.md),
`adr/`, `runbooks/` and `notes/`, with **two deliberate defects**:

- one **broken link** in `runbooks/incident-response.md` (it points to a file
  that does not exist), and
- one **orphan document**, `notes/scratchpad.md` (nothing links to it, it
  links to nothing).

Both are marked with a comment in their source file. Please don't fix them —
they are what the demo detects.

Every output below comes from a real run. Content-derived values (`blake3:…`
revisions, `changeset:…` ids) will differ in your run whenever the files
differ; everything else should match.

## 1. Validate from the CLI

From this directory:

```console
$ lodestar check
  ✗ [LINK-TARGET-MISSING] runbooks/incident-response.md: El enlace apunta a un documento que no existe: «runbooks/escalation.md».

10 documentos · 1 con errores · 0 avisos · NO VÁLIDO
$ echo $?
1
```

The broken link is found; the non-zero exit code is what makes `lodestar
check` usable as a CI gate.

## 2. Work on it as an agent (MCP)

Start the server with this directory as root — no init, no config:

```console
$ lodestar-mcp --root examples/demo
```

The steps below show each tool call (arguments, then the relevant part of the
response) as an MCP client sees them.

### Find documents with a typed query

Which tier-1 services have runbooks, and who is on call? `where` queries the
YAML frontmatter with real types — `service.tier = 1` is a number comparison,
not a string match:

```json
knowledge_search
{"where": "has(service) and service.tier = 1", "include": ["frontmatter.oncall"]}
```

```json
{
  "results": [
    {"path": "runbooks/deploy.md",            "title": "Runbook: deploy",            "frontmatter": {"oncall": "platform"}},
    {"path": "runbooks/incident-response.md", "title": "Runbook: incident response", "frontmatter": {"oncall": "platform"}}
  ],
  "totalApproximate": 2
}
```

### Find the orphan

```json
graph_query
{"operation": "isolated"}
```

```json
{
  "nodes": [{"id": "notes/scratchpad.md", "title": "Scratchpad", "ghost": false}],
  "summary": {"nodeCount": 1, "edgeCount": 0, "truncated": false}
}
```

### Measure impact before touching anything

What would moving `architecture.md` affect?

```json
impact_analyze
{"ref": {"path": "architecture.md"}, "proposedOperation": {"kind": "move"}}
```

```json
{
  "summary": {"directlyAffected": 4, "transitivelyAffected": 7, "risk": "medium", "blockingReferences": 0},
  "recommendations": ["Revisa los 4 enlaces entrantes que apuntan a este documento tras aplicar «move»."]
}
```

Four documents link here directly; seven are in the transitive blast radius.
The agent knows the cost of the change before proposing it.

### Plan, apply, revert

Promote ADR-0002 from `proposed` to `accepted` — first as a dry-run plan. The
workspace already has one (deliberate) error, so the plan is told not to
require a fully valid result:

```json
change_plan
{
  "operations": [{"op": "patch_frontmatter", "path": "adr/0002-event-bus.md", "patch": {"status": "accepted"}}],
  "policy": {"requireValidResult": false, "allowWarnings": true}
}
```

```json
{
  "changeSetId": "changeset:5983d33f…",
  "canApply": true,
  "risk": {"level": "low", "reasons": []},
  "impact": {"affectedCount": 1, "affectedDocuments": ["adr/0002-event-bus.md"]},
  "semanticDiff": {"frontmatterChanges": ["adr/0002-event-bus.md"], "modified": ["adr/0002-event-bus.md"], "created": [], "deleted": [], "moved": []},
  "diagnosticsBefore": {"errors": 1, "warnings": 0},
  "diagnosticsAfter":  {"errors": 1, "warnings": 0}
}
```

Nothing has been written yet. Apply it — the write goes through the full
transactional path (staging, lock, recovery copies, write-ahead journal,
atomic renames, receipt):

```json
change_apply
{"changeSetId": "changeset:5983d33f…"}
```

```json
{
  "applied": true,
  "changedPaths": ["adr/0002-event-bus.md"],
  "receiptId": "5983d33f…"
}
```

The frontmatter patch is surgical: only `status` changed, every other byte of
the document survived. Now undo it with the receipt:

```json
change_revert
{"receiptId": "5983d33f…"}
```

```json
{
  "reverted": true,
  "changedPaths": ["adr/0002-event-bus.md"]
}
```

And the tree is byte-for-byte back where it started:

```console
$ git status --porcelain -- examples/demo
$
```

That round trip — plan without writing, apply with a receipt, revert from the
receipt — is the point: an agent can change this workspace without being able
to half-destroy it.

## Reproduce it yourself

The exact session above is scripted: from the repository root, run
`scripts/demo-smoke.sh` (the same script CI runs to keep this document
honest).
