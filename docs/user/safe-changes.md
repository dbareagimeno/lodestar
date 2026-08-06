# Safe changes: plan, apply, revert

Letting an agent edit your documentation is only reasonable if a bad edit is **visible before it
happens** and **undoable after it happens**. Lodestar splits every change into three tools:

| Tool | What it does | Touches disk |
|---|---|---|
| `change_plan` | Normalizes the operations, simulates the result in memory, and reports the diff, the impact and the diagnostics before and after | No |
| `change_apply` | Publishes one specific plan through staging, a lock, recovery copies, a write-ahead journal and atomic renames | Yes |
| `change_revert` | Restores the state a receipt describes, as a new inverse transaction | Yes |

All three are served only under the `standard` profile; `readonly` hides **and** refuses them (see
[mcp-clients.md](mcp-clients.md#profiles-readonly-and-standard)).

Every request and response on this page comes from a real run against
[`examples/demo/`](../../examples/demo/README.md). Responses are trimmed to the fields under
discussion, and content-derived values (`blake3:…` revisions, `changeset:…` ids, receipt ids) will
differ in your run. Diagnostic and error messages are quoted verbatim and are currently in Spanish
(wrapped here for readability; the engine emits each one on a single line); error **codes** are
stable and in English.

- [A round trip](#a-round-trip)
- [What the plan checks](#what-the-plan-checks)
- [`canApply` is the plan's verdict, not a lock](#canapply-is-the-plans-verdict-not-a-lock)
- [The seven operations](#the-seven-operations)
- [Bulk selections](#bulk-selections)
- [Optimistic concurrency](#optimistic-concurrency)
- [What `change_apply` guarantees](#what-change_apply-guarantees)
- [Receipts and retention](#receipts-and-retention)
- [Crash recovery and its limits](#crash-recovery-and-its-limits)
- [Error codes](#error-codes)
- [Reference](#reference)

## A round trip

Raise the retention of the backup runbook from 30 days to 45. First, plan it. The demo workspace
ships with one deliberate error (a broken link), so the plan is told not to require a fully valid
result — see [`canApply`](#canapply-is-the-plans-verdict-not-a-lock) below:

```json
change_plan
{
  "operations": [{"op": "patch_frontmatter", "path": "runbooks/backup.md", "patch": {"retention_days": 45}}],
  "policy": {"requireValidResult": false, "allowWarnings": true}
}
```

```json
{
  "changeSetId": "changeset:5bc591c4…",
  "planHash": "blake3:5bc591c4…",
  "baseWorkspaceRevision": "blake3:c1d5aee4…",
  "expiresAt": "1785628106",
  "canApply": true,
  "normalizedOperations": [
    {"op": "patch_frontmatter", "path": "runbooks/backup.md", "patch": {"retention_days": 45}}
  ],
  "risk": {"level": "low", "reasons": []},
  "impact": {"affectedCount": 1, "affectedDocuments": ["runbooks/backup.md"]},
  "semanticDiff": {"created": [], "deleted": [], "moved": [], "modified": ["runbooks/backup.md"],
                   "frontmatterChanges": ["runbooks/backup.md"], "bodyChanges": [],
                   "diagnosticsIntroduced": [], "diagnosticsResolved": []},
  "capturedRevisions": {},
  "diagnosticsBefore": {"errors": 1, "warnings": 0, "info": 0},
  "diagnosticsAfter":  {"errors": 1, "warnings": 0, "info": 0}
}
```

Nothing has been written. Apply that exact plan, by id:

```json
change_apply
{"changeSetId": "changeset:5bc591c4…"}
```

```json
{
  "applied": true,
  "receiptId": "5bc591c4…",
  "changedPaths": ["runbooks/backup.md"],
  "previousWorkspaceRevision": "blake3:c1d5aee4…",
  "workspaceRevision": "blake3:b001221b…",
  "validation": {"valid": false, "errors": 1, "warnings": 0}
}
```

The edit is surgical — only the line it names changes, every other byte of the document survives:

```console
$ head -5 runbooks/backup.md
---
schedule: daily
retention_days: 45
last_reviewed: "2026-05-12"
---
```

`validation.valid` is `false` because the workspace still has its pre-existing broken link, not
because this change broke anything: `errors` was `1` before and is `1` after. Now undo it with the
receipt:

```json
change_revert
{"receiptId": "5bc591c4…"}
```

```json
{
  "reverted": true,
  "receiptId": "5bc591c4…-revert",
  "changedPaths": ["runbooks/backup.md"],
  "previousWorkspaceRevision": "blake3:b001221b…",
  "workspaceRevision": "blake3:c1d5aee4…"
}
```

The workspace revision is back to the one the plan was built on, and the file is byte-for-byte what
it was. Note that the revert has its **own** receipt id (`…-revert`): undoing is itself a
transaction, with its own journal and its own recovery copies.

## What the plan checks

`change_plan` does everything except write:

1. **Normalizes** each operation into concrete paths and content. One high-level operation can
   expand into several — a `move` with `rewriteInboundLinks` becomes the move plus one rewrite per
   linking document — and all of them travel as a single change set.
2. **Captures revisions**: the `baseWorkspaceRevision` it planned against, plus (for a bulk
   selection) the `DocumentRevision` of every selected document in `capturedRevisions`.
3. **Simulates** the result in memory and derives the `semanticDiff` (created, deleted, moved,
   modified, frontmatter vs body changes, diagnostics introduced and resolved).
4. **Assesses impact and risk** over the link graph: which documents are affected, how many, and a
   `risk` level with its reasons.
5. **Validates** the hypothetical result: `diagnosticsBefore` and `diagnosticsAfter` let you see
   whether the change repairs, preserves or degrades the workspace.
6. **Stamps identity**: `planHash` is `blake3(baseWorkspaceRevision ‖ normalized operations)`, so
   the same input on the same base always yields the same hash — and a different input never does.
   The `changeSetId` derives from it. The hash does **not** depend on the clock; `expiresAt` does,
   and is not part of it.

A plan is stored and stays applicable for **one hour** (`expiresAt`, seconds since the epoch). After
that, `change_apply` answers `PLAN_EXPIRED` and you plan again.

## `canApply` is the plan's verdict, not a lock

`canApply` answers one question: *would this result satisfy the `policy` I passed to this plan?* It
is `false` when `requireValidResult` is on and the simulated result is not valid, or when
`allowWarnings` is off and the result has warnings.

Read it as advice to the caller, and act on it. It is **not** a safety interlock:
[`contracts/mcp.yml`](../../contracts/mcp.yml) does not state that `change_apply` re-evaluates the
plan's policy, and what `change_apply` does enforce is its own staging gate, described
[below](#what-change_apply-guarantees) — a differential check with different criteria. A client that
ignores `canApply: false` and applies anyway is not being stopped by the policy it declared.

On the demo workspace this is easy to see, because it ships with one error on purpose. Under the
default policy (`requireValidResult: true`, `allowWarnings: true`) every plan comes back with
`canApply: false`, since the *result* still contains the pre-existing broken link:

```json
change_plan
{"operations": [{"op": "patch_frontmatter", "path": "runbooks/backup.md", "patch": {"retention_days": 45}}]}
```

```json
{
  "changeSetId": "changeset:5bc591c4…",
  "canApply": false,
  "diagnosticsBefore": {"errors": 1, "warnings": 0, "info": 0},
  "diagnosticsAfter":  {"errors": 1, "warnings": 0, "info": 0}
}
```

That is the right policy for a workspace you expect to be clean, and the wrong one for a workspace
you are repairing incrementally — which is why the round trip above passes
`requireValidResult: false`. Omitting `policy` entirely is fine and uses the defaults above; so is
sending a partial object — `{"requireValidResult": false}` on its own is enough, and the omitted
field falls back to its own default (`allowWarnings: true`) instead of being rejected.

## The seven operations

Seven universal operations cover every change; there are no document-type-specific ones, because
Lodestar has no document types.

| `op` | Does |
|---|---|
| `create` | Creates a document with optional frontmatter and body |
| `patch_frontmatter` | Merges keys into the frontmatter, preserving untouched lines byte for byte |
| `replace_body` | Replaces the body, keeping the frontmatter |
| `replace_text` | Replaces occurrences of a literal string, optionally asserting how many |
| `edit_section` | Edits the content under a heading path |
| `move` | Moves or renames a document, optionally rewriting inbound links |
| `delete` | Deletes a document, with an explicit policy for inbound links |

Parameter names and shapes live in [`contracts/mcp.yml`](../../contracts/mcp.yml) under
`change_plan`; they are also declared in the tool's `inputSchema`, which your MCP client can show
you. Two behaviours are worth calling out because they are refusals by design:

**`delete` never guesses what to do with inbound links.** If the document has backlinks, the policy
is mandatory:

```json
change_plan
{"operations": [{"op": "delete", "path": "adr/0002-event-bus.md"}],
 "policy": {"requireValidResult": false, "allowWarnings": true}}
```

```text
INVALID_SCHEMA: «adr/0002-event-bus.md» tiene 5 enlaces entrantes, así que «delete» exige elegir
explícitamente «inboundLinksPolicy»: ["reject", "remove_links"]
```

Choosing `reject` explicitly means "refuse if anything links here", and that is what happens:

```text
INBOUND_LINKS_EXIST: el documento «adr/0002-event-bus.md» tiene enlaces entrantes; no se puede borrar
con la política «reject»
```

`remove_links` deletes the document and removes the references to it. Before choosing, ask
`impact_analyze` what a delete would cost.

## Bulk selections

Instead of an `operations` array you can pass a `selection` plus a single `operation`, and the
operation expands to one per selected document. The selection speaks the
[query language](query-language.md); `patch_frontmatter`, `replace_text` and `delete` are the
operations that make sense in bulk.

```json
change_plan
{
  "selection": {"where": "status = \"proposed\""},
  "operation": {"patch_frontmatter": {"reviewed": true}},
  "policy": {"requireValidResult": false, "allowWarnings": true}
}
```

```json
{
  "changeSetId": "changeset:b3b49635…",
  "canApply": true,
  "normalizedOperations": [
    {"op": "patch_frontmatter", "path": "adr/0002-event-bus.md", "patch": {"reviewed": true}}
  ],
  "capturedRevisions": {"adr/0002-event-bus.md": "blake3:595fbe6d…"},
  "impact": {"affectedCount": 1, "affectedDocuments": ["adr/0002-event-bus.md"]}
}
```

`capturedRevisions` is the snapshot the plan took of every selected document: you can see exactly
which files, at which revisions, the plan is built on before applying anything. A selection that
matches nothing produces an empty plan, not an error — but a query that cannot be evaluated (a type
error) aborts the plan with `INVALID_SCHEMA`, with the same message `knowledge_search` would give.

## Optimistic concurrency

Lodestar does not assume it is the only writer. Revisions are content hashes, and you can pin them
at two levels:

| Parameter | Where | Meaning |
|---|---|---|
| `operations[].expectedRevision` | `change_plan` | The `DocumentRevision` you believe that document is at |
| `expectedWorkspaceRevision` | `change_plan`, `change_apply`, `change_revert` | The workspace revision you believe is current |

Both are optional; when omitted, the current value is adopted. When supplied and stale, the call is
refused **before anything is written**:

```json
change_plan
{"operations": [{"op": "patch_frontmatter", "path": "runbooks/backup.md",
                 "patch": {"retention_days": 45},
                 "expectedRevision": "blake3:00000000000…"}]}
```

```text
REVISION_CONFLICT: «runbooks/backup.md» ya no está en la revisión «blake3:00000000…» que declara la
operación (ahora es «blake3:ed963cd8…»). Vuelve a leerlo (knowledge_get) y replanifica
```

```json
change_plan
{"expectedWorkspaceRevision": "blake3:11111111111…",
 "operations": [{"op": "patch_frontmatter", "path": "runbooks/backup.md", "patch": {"retention_days": 45}}]}
```

```text
REVISION_CONFLICT: el workspace ya no está en la revisión esperada: «expectedWorkspaceRevision» es
blake3:11111111… y la actual es blake3:c1d5aee4…. Vuelve a leer el estado (workspace_status) y replanifica
```

You get the same protection without asking for it. `change_apply` recomputes the plan hash against
the workspace as it is **now**; if the base moved between plan and apply, the plan is stale and
nothing is written:

```text
PLAN_STALE: el conocimiento cambió bajo el plan «changeset:5bc591c4…»: se planificó sobre la revisión
blake3:c1d5aee4… y el workspace está en blake3:79674e5e…, así que no se ha escrito nada. Vuelve a
llamar a change_plan sobre el estado actual
```

And `change_revert` refuses to undo on top of someone else's work. Edit an affected file after the
apply and the revert stops rather than overwriting it:

```text
WRITE_CONFLICT: el conocimiento cambió después del apply «changeset:c4ec31a2…» (quedó en
blake3:79674e5e… y ahora está en blake3:7daf5bea…), así que revertir pisaría ese cambio: no se ha
restaurado nada
```

A `WRITE_CONFLICT` is terminal for that transaction: re-read the state and plan again.

## What `change_apply` guarantees

The apply path runs in a fixed order, and every step before the first rename can still refuse the
whole transaction without having touched your Markdown:

1. Load the persisted plan — `PLAN_EXPIRED` if it timed out, `PLAN_STALE` if it is gone.
2. Check `expectedWorkspaceRevision`, if given.
3. Recompute the plan hash against the current base — `PLAN_STALE` if the workspace moved.
4. Take the **publication lock**, so two publishers cannot interleave.
5. Compute the real affected set (created, modified, deleted).
6. Check every affected path against the configured writable roots — `PERMISSION_DENIED` otherwise.
7. Stage the result and run the **differential validation gate**: with the defaults, a change may
   not introduce diagnostics the workspace did not already have (`rejectNewErrors`), while existing
   problems are tolerated so partial repairs remain possible (`allowExistingErrors`). A violation is
   `INVALID_RESULT`.
8. Re-verify the base under the lock.
9. Write **durable recovery copies** of everything about to change, each with its size and `blake3`
   fingerprint.
10. Write the **write-ahead journal**, fsynced.
11. Persist the receipt — before the point of no return.
12. Publish with **atomic renames**, one file at a time, through the single writer.
13. Seal: promote the receipt, drop the journal, clean staging, keep the recovery copies.

Two consequences of that order are contract, not implementation detail:

- **Publishing implies a receipt.** No step after the first rename can turn a published transaction
  into an error: sealing, cleanup, retention and even the post-apply `validation` are best-effort
  and report to `stderr`. An apply that published answers success. The corner case that follows is
  worth knowing: `validation.valid == false` with `errors == 0 && warnings == 0` means *the verdict
  could not be computed*, not *the result is invalid*.
- **A transaction that did not publish leaves no receipt**, so there is nothing to revert and
  `change_revert` on it answers `PLAN_EXPIRED`.

If another process changes the canonical files inside the publication window, the apply compares
what it backed up against what is on disk and aborts with `WRITE_CONFLICT` **before the first
rename**, naming the divergent paths.

## Receipts and retention

A receipt is what makes an apply undoable. `workspace_status` lists the ones still on disk, newest
first, so losing a `receiptId` does not put the undo out of reach:

```json
workspace_status
{}
```

```json
{
  "receipts": [
    {"receiptId": "5bc591c4…",        "changeSetId": "changeset:5bc591c4…", "resultRevision": "blake3:b001221b…", "changedPathCount": 1},
    {"receiptId": "f9b7d4f1…-revert", "changeSetId": "changeset:f9b7d4f1…", "resultRevision": "blake3:c1d5aee4…", "changedPathCount": 1}
  ],
  "recovery": {"pendingTransaction": false}
}
```

Each entry carries just enough to choose which one to undo; `change_revert` reads the full receipt.

Retention is bounded two ways, both configurable in `.lodestar/config.yaml` under `transactions`:
`maximumReceipts` (20 by default) and `retainReceiptsFor` (`"24h"` by default). The collector runs
**after each apply and each revert**, under the publication lock. Purging a receipt also removes its
recovery copies, so a purged transaction is no longer revertible — `change_revert` then answers:

```text
PLAN_EXPIRED: no hay recibo «deadbeef»: esa transacción ya no es reversible (el recibo nunca existió o
la retención lo purgó)
```

Two practical notes. Under `readonly` the collector never runs, because nothing is ever published.
And receipts and recovery copies live under `.lodestar/runtime/`, which belongs to the engine: it is
disposable state, not history, and `change_revert` is one step back — not a version control system.
If you need history, keep the workspace in git.

## Crash recovery and its limits

If the process dies mid-publication, the journal is still on disk. When the workspace is next opened
— in practice, at the next `change_plan` — the engine recovers **before** doing anything else,
deterministically: either it **completes** the transaction
(if it had passed the point of no return) or it **restores** the previous state from the recovery
copies. `workspace_status.recovery.pendingTransaction` reports the situation, and `change_plan`
resolves it before planning anything new, so in normal operation you will rarely see it.

What is guaranteed: **while the recovery copies verify**, the canonical files converge to one of the
two edges — the state before the transaction or the state after it — never to a half-written
mixture. Documents are replaced by atomic rename, so no reader ever sees a partial file. A completed
transaction keeps its recovery copies, so it stays revertible even if the crash happened between the
last rename and the sealing.

What is **not** guaranteed, stated plainly:

- **If a recovery copy does not verify** — missing, unreadable, or failing its size and `blake3`
  fingerprint — that transaction is not restored. Nothing is written from a copy that does not
  verify. Its journal and its copies are *moved* to `.lodestar/runtime/journal/quarantine/<txnId>/`
  (nothing is deleted: it is forensic material), recovery continues with the other pending journals,
  and the call fails with `RECOVERY_FAILED`, whose message names the quarantine path. Because the
  journal is no longer where the gate looks, the workspace becomes writable again — one unreadable
  file no longer locks it forever.
- **Quarantined material is not surfaced anywhere else.** No field of `workspace_status` reports it.
  If you do not read the error message or `stderr`, you will not know it is there.
- **While an unresolved recovery is pending, writes are refused** with
  `WORKSPACE_RECOVERY_REQUIRED`.
- **Nothing protects you from edits outside the transaction.** Recovery restores what the
  transaction changed, not what someone else did to the workspace meanwhile — that is what the
  `WRITE_CONFLICT`s described above are for.

## Error codes

Every failure comes back as a stable code in English plus a message in Spanish, in the form
`CODE: message`. The ones you meet on the change path:

| Code | Means | Do |
|---|---|---|
| `REVISION_CONFLICT` | A revision you pinned is no longer current | Re-read and plan again |
| `PLAN_STALE` | The plan is gone, or the workspace moved under it | Plan again |
| `PLAN_EXPIRED` | The plan timed out, or the receipt no longer exists | Plan again; the transaction is no longer revertible |
| `WRITE_CONFLICT` | Another publisher holds the lock, the base moved under the lock, or a file changed inside the publication window (or after the apply you are reverting) | Terminal for that transaction: re-read and plan again |
| `INVALID_RESULT` | The staging gate refused: the result introduces diagnostics the workspace did not have | Fix the operation, or relax the policy in the config |
| `INBOUND_LINKS_EXIST` | `delete` with `reject` on a document with backlinks | Choose another policy, or remove the links first |
| `PERMISSION_DENIED` | An affected path is outside the writable roots | Check `writableRoots` / `referenceRoots` in the config |
| `WORKSPACE_RECOVERY_REQUIRED` | An interrupted transaction is still pending and was not resolved | Call `change_plan`: it runs recovery first, and reports `RECOVERY_FAILED` if it cannot |
| `RECOVERY_FAILED` | An interrupted transaction could not be restored; material is quarantined | Read the path in the message before doing anything else |
| `INVALID_SCHEMA` | The request itself is wrong — unknown operation, missing parameter, unusable query | Fix the call; the message names the parameter |
| `DOCUMENT_NOT_FOUND` | An operation targets a document that does not exist | Check the path with `knowledge_search` |

## Reference

The authority on parameters, return shapes and the exact conditions of every error is
[`contracts/mcp.yml`](../../contracts/mcp.yml) — the entries for `change_plan`, `change_apply`,
`change_revert` and `workspace_status`. It is written in Spanish, like the rest of the internal
material. For selecting documents to change, see [query-language.md](query-language.md); for a full
agent session end to end, see [`examples/demo/README.md`](../../examples/demo/README.md).
