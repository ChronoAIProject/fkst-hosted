---
"fkst-hosted": minor
---

Add `GET /api/v1/orgs` listing the organizations the caller belongs to (sourced from NyxID with the caller's own delegated token), so clients can discover the `org_id` values they may scope packages, goals, and sessions to. Owner-only mode (NyxID not configured) returns `200 []`; a NyxID outage is fail-closed `503`.
