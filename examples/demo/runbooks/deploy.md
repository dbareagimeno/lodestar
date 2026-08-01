---
service:
  name: atlas-api
  tier: 1
oncall: platform
last_reviewed: "2026-06-30"
---

# Runbook: deploy

Ordering matters (see [ADR-0002](../adr/0002-event-bus.md)): bus first, then
writers, collector last.

1. Announce the deploy in the ops channel.
2. Roll the bus nodes one at a time; wait for green health checks.
3. Roll the writer pool.
4. Roll the collector.
5. Watch ingest lag for 15 minutes.

If anything looks wrong, stop and follow the
[incident response runbook](incident-response.md). Context for the whole
system lives in the [architecture page](../architecture.md).
