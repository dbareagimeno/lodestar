---
service:
  name: atlas-api
  tier: 1
severity_levels: [1, 2, 3]
oncall: platform
---

# Runbook: incident response

1. Acknowledge the page and open an incident channel.
2. Classify severity (1 is highest; levels are listed in the frontmatter).
3. Stabilize first, diagnose second. Bus lag? Check
   [ADR-0002](../adr/0002-event-bus.md) for the intended behaviour under
   back-pressure.
4. If the incident started right after a deploy, roll back following the
   [deploy runbook](deploy.md) in reverse order.

<!-- The link below is DELIBERATELY broken: escalation.md does not exist.
     It is here so that `lodestar check` has something real to report in the
     demo. Do not "fix" it. -->
5. Severity 1 incidents page leadership automatically — the contact chain is
   in the [escalation policy](escalation.md).
