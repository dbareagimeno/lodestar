---
status: accepted
date: "2025-11-04"
tags: [architecture, storage]
deciders: [alice, bruno]
---

# ADR-0001: Markdown files are the source of truth

## Context

Team knowledge was split between a wiki, tickets and tribal memory. We want
one place that survives tool churn and works offline.

## Decision

All durable knowledge lives as Markdown files in this repository. Anything
derived from them (indexes, caches, rendered sites) must be reconstructible
and disposable.

## Consequences

- Reviews of knowledge changes ride the normal PR flow.
- Tooling must treat the `.md` files as canonical — see
  [ADR-0002](0002-event-bus.md) for the first decision recorded this way.
- The [backup runbook](../runbooks/backup.md) covers the repository, not any
  derived store.
