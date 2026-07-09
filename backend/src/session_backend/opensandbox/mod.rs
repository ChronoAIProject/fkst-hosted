//! OpenSandbox sandbox-lifecycle HTTP client (issue #416).
//!
//! A thin, hand-rolled reqwest client for the upstream OpenSandbox *sandbox
//! lifecycle* API — the create / get / list / patch-metadata / delete verbs a
//! future OpenSandbox-backed [`super::SessionBackend`] will drive. This issue lands
//! ONLY the transport + wire DTOs: there is no `SessionBackend` impl, no execd
//! endpoints, and no retry/config here.
//!
//! ## Wire contract (grounded, not guessed)
//! The DTOs + endpoints are pinned to the upstream OpenAPI spec
//! `specs/sandbox-lifecycle.yml` at tag `server/v0.2.1`
//! (<https://github.com/opensandbox-group/OpenSandbox/blob/server/v0.2.1/specs/sandbox-lifecycle.yml>;
//! the file's own `info.version` is `0.1.0`), consulted 2026-07-09. Points confirmed
//! against that revision — several differ from first-guess defaults:
//! - list envelope is `{items, pagination:{page,pageSize,totalItems,totalPages,hasNextPage}}`
//!   — page-NUMBER pagination, NOT a `nextPageToken` cursor;
//! - lifecycle facts are nested under `status.{state,reason,message}`, not top-level;
//! - `resourceLimits` is a free-form `map<string,string>` (cpu / memory / gpu / …);
//! - image pull `auth` is `{username, password}`;
//! - the metadata PATCH body is `application/json` (RFC 7396 *semantics*), NOT the
//!   `application/merge-patch+json` media type;
//! - the API key rides the `OPEN-SANDBOX-API-KEY` header;
//! - create answers `202`, delete answers `204`, both `404` when the id is unknown.

pub mod dto;
pub mod lifecycle;

pub use dto::*;
pub use lifecycle::OsbLifecycleClient;
