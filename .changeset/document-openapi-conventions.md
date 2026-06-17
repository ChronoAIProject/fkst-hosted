---
---

Document the OpenAPI `/openapi.json` conventions in `CLAUDE.md`: how to keep the dynamically generated spec correct when adding or changing a public endpoint (annotate handlers with `#[utoipa::path]`, register via `utoipa-axum`'s `OpenApiRouter`/`routes!`, derive `ToSchema`/`IntoParams`), the off-by-default `fkst-shared` `schema` feature that keeps the worker free of `utoipa`, the public-surface-only scope, component-name-collision handling, and the `utoipa`/`utoipa-axum` version pins. Docs only — no code or behaviour change. Closes #234.
