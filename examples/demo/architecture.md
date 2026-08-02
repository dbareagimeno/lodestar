# Architecture

Atlas ingests telemetry events, buffers them in a queue and writes them to
cold storage. Three moving parts:

1. **Collector** — receives events over HTTP, validates and enqueues them.
2. **Bus** — the queue between collector and writers; see
   [ADR-0002](adr/0002-event-bus.md) for why we chose it.
3. **Writer pool** — drains the bus into storage in batches.

Operational entry points live in the runbooks: [deploy](runbooks/deploy.md)
and [incident response](runbooks/incident-response.md).

Back to the [overview](overview.md).
