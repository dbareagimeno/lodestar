---
status: proposed
date: "2026-01-09"
tags: [architecture, queue]
review:
  due: "2026-02-01"
  owner: alice
---

# ADR-0002: A single event bus between collector and writers

## Context

The collector used to write straight to storage; back-pressure during traffic
spikes dropped events. The [architecture](../architecture.md) now calls for a
buffer in between.

## Decision

Introduce one durable queue (the "bus") between collector and writer pool.
Writers consume in batches; the collector never blocks on storage.

## Consequences

- Deploys must roll the bus first — the ordering is spelled out in the
  [deploy runbook](../runbooks/deploy.md).
- Incident triage gains a new failure mode (bus lag); see the
  [incident response runbook](../runbooks/incident-response.md).
