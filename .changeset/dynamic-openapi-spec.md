---
---

Serve a dynamically generated OpenAPI 3 specification at `GET /openapi.json` on the control plane. The document is assembled at runtime from the live Axum routes and Rust types via `utoipa` + `utoipa-axum` — there is no checked-in static spec, and the route registration is the documented path (no drift). It covers the public surface (`/api/v1/*`, `/health`, `/metrics`, and the GitHub App webhook when configured) and excludes the fleet-only `/internal/v1/*` worker protocol. The shared wire types are documented full-fidelity behind a new off-by-default `schema` feature on `fkst-shared`, so the worker stays free of `utoipa`. Closes #232.
