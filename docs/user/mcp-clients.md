# Connecting an MCP client

`lodestar-mcp` is a local process that speaks JSON-RPC over `stdio`. Any MCP client that can launch
a subprocess can use it: you tell the client the command, the client owns the process lifecycle.

Prerequisite: `lodestar-mcp` on your `PATH` (see [quickstart.md](quickstart.md)). Every output on
this page comes from a real run against [`examples/demo/`](../../examples/demo/README.md).

- [Claude Code](#claude-code)
- [Any client configured with JSON](#any-client-configured-with-json)
- [`--root`: when to pass it, when to omit it](#--root-when-to-pass-it-when-to-omit-it)
- [Profiles: `readonly` and `standard`](#profiles-readonly-and-standard)
- [A tour of the ten tools](#a-tour-of-the-ten-tools)
- [Troubleshooting](#troubleshooting)

## Claude Code

One line:

```bash
claude mcp add lodestar -- lodestar-mcp --root /absolute/path/to/project
```

Everything after `--` is the command and its arguments. Add `--scope project` to write the entry
into a `.mcp.json` that you can commit and share with your team; the default scope is local to you.
With `--scope project`, this is what it writes:

```console
$ claude mcp add lodestar --scope project -- lodestar-mcp --root /absolute/path/to/project
Added stdio MCP server lodestar with command: lodestar-mcp --root /absolute/path/to/project to project config
$ cat .mcp.json
{
  "mcpServers": {
    "lodestar": {
      "type": "stdio",
      "command": "lodestar-mcp",
      "args": [
        "--root",
        "/absolute/path/to/project"
      ],
      "env": {}
    }
  }
}
```

`claude mcp list` shows the server and its health. A project-scoped server appears as
*Pending approval* until you accept it once from inside `claude`.

## Any client configured with JSON

The generic `mcpServers` shape — the same object Claude Code just wrote — works in any client that
launches `stdio` servers:

```json
{
  "mcpServers": {
    "lodestar": {
      "command": "lodestar-mcp",
      "args": ["--root", "/absolute/path/to/project", "--profile", "readonly"]
    }
  }
}
```

Two things matter for a well-behaved client:

- **`stdout` is JSON-RPC and nothing else.** Never wrap the binary in a script that prints to
  `stdout`; it will corrupt the stream.
- **Logs go to `stderr`.** On startup the server prints one line there, which is where you look
  first when a client says the server failed:

```console
$ lodestar-mcp --root examples/demo 1>/dev/null
lodestar-mcp: escuchando JSON-RPC en stdio (root=/…/examples/demo, profile=Standard)
```

(The absolute root is elided above. The server then waits for JSON-RPC on `stdin`; `Ctrl-C` stops
it.)

There is no positional argument and no other transport: `lodestar-mcp [--root <dir>]
[--profile readonly|standard]` is the whole startup surface.

### Poking the server by hand

Because the protocol is line-delimited JSON on `stdio`, you can drive it from a shell — which is how
every console block below was produced. Three request files, run from `examples/demo/`:

```bash
cat > requests.jsonl <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
EOF

cat > status.jsonl <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"workspace_status","arguments":{}}}
EOF

cat > call-change-plan.jsonl <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"change_plan","arguments":{"operations":[{"op":"patch_frontmatter","path":"adr/0002-event-bus.md","patch":{"status":"accepted"}}]}}}
EOF
```

The server reads until `stdin` closes, so piping a file in and a `jq` filter out gives you one
complete exchange per run. (`2>/dev/null` below just drops the startup log line.)

## `--root`: when to pass it, when to omit it

`--root` is the workspace root. It is resolved once at startup and stays fixed for the whole
session; every path in every request and response is relative to it.

- **Pass it** when the client starts the process from somewhere else — an editor, a desktop app, a
  daemon. Use an absolute path: you rarely control the working directory a client launches from.
- **Omit it** when the process already starts inside the project. Without `--root`, the root is the
  process's current directory, which makes `cd my-project && lodestar-mcp` work with no arguments
  at all.

Any directory qualifies. There is no `init`, no `.lodestar/` requirement, no index file and no
mandatory frontmatter: point it at a documentation repository, a monorepo's `docs/`, or a folder of
notes.

## Profiles: `readonly` and `standard`

`--profile` decides whether the session can change anything. `standard` is the default.

| Profile | Tools served | Change tools |
|---|---|---|
| `readonly` | 7 read and verification tools | Hidden **and** refused |
| `standard` | all 10 | Available |

The three change tools are `change_plan`, `change_apply` and `change_revert`. Under `readonly` they
do not appear in `tools/list`:

```console
$ lodestar-mcp --root . --profile readonly < requests.jsonl | jq -c 'select(.id==2)|.result.tools|map(.name)'
["workspace_status","knowledge_search","knowledge_get","metadata_inspect","knowledge_check","graph_query","impact_analyze"]

$ lodestar-mcp --root . < requests.jsonl | jq -c 'select(.id==2)|.result.tools|map(.name)'
["workspace_status","knowledge_search","knowledge_get","metadata_inspect","knowledge_check","graph_query","impact_analyze","change_plan","change_apply","change_revert"]
```

Hiding is not the guarantee — refusing is. A client that calls a change tool anyway, because it
cached an old tool list or because a model guessed the name, is rejected before the tool ever runs:

```console
$ lodestar-mcp --root . --profile readonly < call-change-plan.jsonl | jq -c 'select(.id==2)'
{"error":{"code":-32602,"message":"tool desconocida: change_plan"},"id":2,"jsonrpc":"2.0"}
```

The profile is also visible in the data, so an agent can adapt instead of guessing. Same call, the
two profiles:

```console
$ lodestar-mcp --root . < status.jsonl | jq -c 'select(.id==2)|.result.structuredContent.capabilities'
{"externalReferences":true,"revert":true,"schemas":false,"transactions":true,"writes":true}

$ lodestar-mcp --root . --profile readonly < status.jsonl | jq -c 'select(.id==2)|.result.structuredContent.capabilities'
{"externalReferences":true,"revert":false,"schemas":false,"transactions":false,"writes":false}
```

Use `readonly` for exploration, review and anything running unattended; switch to `standard` when
you want the agent to be able to propose and apply changes.

## A tour of the ten tools

The `initialize` response carries server instructions describing these ten as a recommended
ten-step flow, in this order. Below is what each one is *for*; for parameters, return shapes, error
codes and the exact semantics, the authority is
[`contracts/mcp.yml`](../../contracts/mcp.yml) (written in Spanish, like the rest of the internal
material), and a worked session lives in
[`examples/demo/README.md`](../../examples/demo/README.md).

### Orient

**`workspace_status`** — *Where am I, and what am I allowed to do?* Returns the active
configuration, the profile's capabilities, the overall verdict and aggregate counts, whether a
publication was interrupted, and which receipts are still available to revert. Call it first in
every session.

```console
$ lodestar-mcp --root . < status.jsonl | jq 'select(.id==2)|.result.structuredContent|{valid, counts}'
{
  "valid": false,
  "counts": {
    "dangling": 1,
    "documents": 10,
    "errors": 1,
    "isolated": 1,
    "links": 30,
    "warnings": 0
  }
}
```

### Read

**`knowledge_search`** — *Which documents are relevant?* Finds documents by free text and by the
typed query language (`where` as a string, `filter` as JSON; both can be combined). It returns
paths, titles, snippets and revisions — never full bodies — with cursor pagination, and `include:
["frontmatter.<field>"]` projects specific metadata so you do not need one read per result.

**`knowledge_get`** — *Give me this document.* Retrieves one document with a selective `include`
(`frontmatter`, `body`, `revision`, `outgoingLinks`, `backlinks`, `diagnostics`); anything not asked
for is not populated. `sections` narrows the body to specific headings, so an agent can read one
section instead of a whole file.

**`metadata_inspect`** — *What conventions does this project actually use?* Two modes: `catalog`
lists every frontmatter field with how many documents carry it and which types it takes; `field`
inspects one field — presence, absence, types and frequent values. It is how an agent learns an
unfamiliar repository without you maintaining a schema.

### Analyze

**`graph_query`** — *How is this connected?* One tool for eight operations over the link graph:
`backlinks`, `outgoing`, `neighborhood` (with `depth` and `direction`), `isolated` (documents with
no internal links in or out), `dangling` (links with no target), `path_between`, `cycles` and
`components`.

**`impact_analyze`** — *What would break if I moved or deleted this?* Evaluates a hypothetical
`move` or `delete` on one document without touching disk: directly and transitively affected
documents, structural blockers, a risk level, and recommendations.

**`knowledge_check`** — *Is the knowledge still interpretable?* Audits with a scope (`workspace`,
`document`, `paths`, or `affected` — the neighbourhood of some documents) and a minimum severity.
It is the same verdict `lodestar check` gives on the command line, from the same engine.

### Change

**`change_plan`** — *Show me the change before it happens.* Normalizes the proposed operations,
simulates them in memory and validates the result. Returns a single change set with the normalized
operations, the semantic diff, risk, impact, the diagnostics before and after, and a deterministic
id. It writes nothing.

**`change_apply`** — *Publish that exact plan.* Takes the `changeSetId` from a plan and applies it
through the full transactional path (staging, lock, recovery copies, write-ahead journal, atomic
renames, receipt). It refuses a plan that has expired or whose workspace changed underneath it, and
refuses writes outside the configured writable roots. Returns a receipt with the before and after
revisions.

**`change_revert`** — *Undo that transaction.* Takes the `receiptId` from an apply and restores the
previous state from the recovery copies, as an inverse transaction with its own journal. It requires
the receipt to still exist and the affected files not to have been altered since the apply.

Optimistic concurrency runs through all three: plans and applies accept an
`expectedWorkspaceRevision`, and individual operations accept a per-document `expectedRevision`, so
a file edited by a human between plan and apply causes a rejected write rather than a silent
overwrite.

## Troubleshooting

**The client reports that the server exited immediately.** Read `stderr`. Exit code `2` means a bad
argument (`lodestar-mcp: argumento no reconocido «…»`); exit code `3` means the root could not be
resolved or the workspace could not be opened — most often a relative `--root` interpreted against a
working directory you did not expect, or an invalid `.lodestar/config.yaml`.

**A tool is missing from the list.** Check the profile: `readonly` serves seven tools. `capabilities`
in `workspace_status` tells you which mode you are in.

**Paths are not found.** Every path in the wire is relative to the root, uses `/` separators, and
never contains `..` or a leading `/`. If a document lives outside the root, it is outside the
workspace — restart the server with a root that contains it.

**The response says the workspace has an unfinished transaction.** A previous publication was
interrupted. Nothing is lost and nothing needs a manual fix: the next `change_plan` completes or
undoes it before planning anything new.
