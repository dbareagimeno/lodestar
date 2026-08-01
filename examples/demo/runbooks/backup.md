---
schedule: daily
retention_days: 30
last_reviewed: "2026-05-12"
---

# Runbook: backups

Per [ADR-0001](../adr/0001-markdown-source-of-truth.md), the repository is the
source of truth: backing up the repo backs up the knowledge.

1. The nightly job mirrors the repository to cold storage.
2. Retention is `retention_days` (see frontmatter).
3. Restore drill: once a quarter, restore yesterday's mirror to a scratch
   machine and open the [overview](../overview.md) from it.
