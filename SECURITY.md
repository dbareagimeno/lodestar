# Security Policy

## Reporting a vulnerability

**Please do not open a public issue for a security problem.**

The primary channel is GitHub Private Vulnerability Reporting, which keeps the report private until
a fix is out:

**<https://github.com/dbareagimeno/lodestar/security/advisories/new>**

If that form is unavailable to you for any reason, email **dbareagimeno@icloud.com** instead.

A useful report contains: the version (or commit) you tested, your platform, the smallest workspace
or input that triggers the problem, the exact command or MCP call, and what an attacker gets out of
it. A proof of concept is welcome; a patch is not required.

Lodestar is maintained by one person in their own time. Reports are handled on a best-effort basis
and there is no guaranteed response time — that is a statement of fact, not an SLA. You will get an
acknowledgement as soon as the report is seen, and the fix will ship in a release with credit to
you unless you prefer otherwise.

## Supported versions

Only the **latest published release** receives security fixes:
<https://github.com/dbareagimeno/lodestar/releases/latest>. Older tags are kept for history and are
not patched. The `main` branch is fixed as work happens.

## What Lodestar is, and what that means for its threat model

Lodestar is a **local engine**. It runs as a one-shot CLI or as an MCP server speaking over
`stdio` on the user's own machine. It makes **no network requests**, has **no telemetry**, no
auto-update, no server component, no accounts and no authorization layer of its own. Nothing leaves
the machine because of Lodestar. This narrows the interesting attack surface considerably, and it is
worth being explicit about what remains.

**The inputs that matter are the Markdown files and the tool calls.** Both come from the user's
environment, but neither is necessarily trustworthy: a workspace can be a repository cloned from
somewhere else, and an agent can be acting on content written by a third party.

In scope — please report:

- **Parsing.** A Markdown document, YAML frontmatter or query expression that makes Lodestar panic,
  hang, or consume memory or CPU out of proportion to its size. The domain core carries
  `#![forbid(unsafe_code)]`, so memory-safety bugs would come from dependencies — report those too.
- **Path traversal.** Every path that reaches disk is meant to pass through the `RelPath` newtype,
  which rejects absolute paths and `..` and is the single chokepoint for this class of bug
  (invariant #6 in [`CLAUDE.md`](CLAUDE.md)). Any way to make Lodestar read or write **outside the
  workspace root** — through a link target, a document path in a change set, a symlink, or an
  ignore file — is a vulnerability.
- **The write path.** The transactional publication (staging, lock file, write-ahead journal,
  atomic renames, backups) is supposed to leave the workspace either in the old state or in the new
  one. A sequence that corrupts a document, destroys content the operator did not target, or leaves
  backup or journal material somewhere it should not be, is a vulnerability.
- **Diagnostic output.** Any content of the workspace that ends up somewhere it was not asked to
  go, including through the JSON or SARIF output of `lodestar check`.

Out of scope:

- **What an agent does with the tools it was granted.** The `standard` profile can modify files:
  that is its purpose. If you want an agent that cannot write, run the server with
  `--profile readonly`, which both hides and rejects the three change tools. Choosing to give a
  writing profile to an agent operating on untrusted content is a deployment decision, not a
  vulnerability in the engine.
- **Unsigned release binaries.** The published binaries are not signed or notarized; this is known
  and documented in [`RELEASING.md`](RELEASING.md), and signing is deferred rather than dismissed.
  Releases published after `v0.5.0` ship a `SHA256SUMS-<target>.txt` asset next to each archive:
  verify it before unpacking.
- **Vulnerabilities in your MCP client, agent or editor**, which should be reported to their
  maintainers.
- **The derived cache.** `.lodestar/index.db` is a rebuildable cache, not a security boundary: the
  Markdown files on disk are the only source of truth, and the cache can be deleted at any time.
