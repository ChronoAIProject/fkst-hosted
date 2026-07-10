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
//!
//! ## execd sub-client (issue #417)
//! [`execd::ExecdClient`] drives the in-sandbox exec daemon (execd) THROUGH the
//! lifecycle proxy (`/v1/sandboxes/{id}/proxy/44772…`): upload files, probe file
//! info (the restart-wipe check), and run + poll + tail commands.
//! [`token::derive_execd_token`] deterministically derives its per-session access
//! token. The execd HTTP contract is pinned to the upstream OpenAPI spec
//! `specs/execd-api.yaml` at tag `server/v0.2.1`, consulted 2026-07-09; two
//! source-grounded facts that the spec alone does not spell out are cited from the
//! execd Go server at that same tag:
//! - the command id is the `text` of the leading `init` `ServerStreamEvent`
//!   (`sse.go` `OnExecuteInit`; `command.go` keys the command status by that id);
//! - `POST /files/upload`'s `mode` is octal DIGITS, read as `ParseUint(Itoa(mode),
//!   8)` (`utils.go`), so the real bits `0o400` travel as the integer `400`.

pub mod backend;
pub mod dto;
pub mod execd;
pub mod lifecycle;
pub mod token;

pub use backend::OsbBackend;
pub use dto::*;
pub use execd::{AuthProbeOutcome, ExecdClient};
pub use lifecycle::OsbLifecycleClient;
pub use token::derive_execd_token;
