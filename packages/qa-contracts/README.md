# Local QA Contracts

`packages/qa-contracts` owns the authoritative, side-effect-free contract source
shared by Hosted and the trusted-input Local QA Host profile. JSON Schema Draft
2020-12 under `contracts/` is the language-neutral source of truth, and
`contracts/registry.json` is the only supported local name-to-schema mapping.
Rust and TypeScript conformance implementations are intentionally deferred to
dependent issues after this source wave merges.

The immutable hardened Local QA Runtime v1 baseline retains
`RuntimeScopedMeta` and `runtime_instance_id`. Human decision #5729 approved the
separate `local_qa_host_mvp` profile supplement, whose canonical names are
`HostScopedMeta` and `host_instance_id`; this package does not modify,
supersede, or add compatibility aliases to the hardened baseline.

P0-02 defines only foundational scalars, metadata, references, exact-object and
strict-union validation mechanics, strict raw JSON admission, RFC 8785/JCS
canonical bytes, and the `contract_content/v1` SHA-256 projection. It does not
define endpoints, authorization, state, resources, runtime behavior, signature
verification, or testing-package behavior.

Schema versions use `qa.<lowercase-kebab-name>/v<positive-major>`. Consumers
must resolve schema and type names through `contracts/registry.json`; runtime
network fetches, external `$ref` resolution, and schema discovery outside this
package are forbidden. Unsupported majors and closed enum values fail closed.
`ProjectionSpecimen` and `StrictUnionSpecimen` are fixture-only harness types,
not public runtime payloads.

The checked-in conformance sources are:

- `fixtures/rfc8785-v1.json` for strict raw JSON admission, canonical UTF-8
  bytes, and SHA-256 vectors;
- `fixtures/qa/contract-foundation-v1.json` for foundation validation,
  exact-object/union behavior, and root-only digest projection vectors.

Canonicalization and digest implementations must accept only opaque immutable
values returned by strict admission and schema validation.
`contract_content/v1` removes only root `content_digest` and, when present,
root `signature`; identically named nested fields are preserved.
