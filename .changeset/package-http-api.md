---
"fkst-hosted": minor
---

Package HTTP API: POST /api/v1/packages (201 + Location, 409 on duplicate), GET /api/v1/packages (sorted name list), and GET /api/v1/packages/{name} (full package with RFC3339 timestamps), with a 16 MiB body cap and every invalid request answered as a 400 in the canonical error envelope.
