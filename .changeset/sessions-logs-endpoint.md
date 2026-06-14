---
"fkst-hosted": minor
---

Add `GET /api/v1/sessions/{id}/logs` returning a best-effort tail of a session's engine child logs (bounded by `log_tail_lines` and a 16 KiB cap), authorized exactly like the session read. Pending, idle, and cross-pod cases return `200` with `logs: null` (and an explanatory `note` for the cross-pod case), never a `404`. Supports `?lines=` clamped to `1..=log_tail_lines`.
