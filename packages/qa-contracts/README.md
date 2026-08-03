# Local QA Contracts

`packages/qa-contracts` owns the authoritative, side-effect-free contract source
shared by Hosted and the trusted-input Local QA Host profile. JSON Schema Draft
2020-12 under `contracts/` is the language-neutral source of truth, and
`contracts/registry.json` is the only supported local name-to-schema mapping.
The checked-in TypeScript package and Rust crate consume these same sources and
shared fixture corpus; neither implementation maintains language-specific
golden values.

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

The implementations are:

- `src/index.ts`, a private ESM TypeScript package locked by the package-local
  `package-lock.json`;
- `rust/src/lib.rs`, the publish-disabled `fkst-qa-contracts` library, built as
  an external member of the Local QA Runtime workspace and locked by
  `apps/local-qa-runtime/Cargo.lock`.

Canonicalization and digest functions accept only opaque values returned by
strict admission or schema validation. `contract_content/v1` removes only root
`content_digest` and, when present, root `signature`; identically named nested
fields are preserved.

Run the TypeScript conformance checks from the repository root:

```bash
npm --prefix packages/qa-contracts ci --ignore-scripts
npm --prefix packages/qa-contracts run --ignore-scripts typecheck
npm --prefix packages/qa-contracts run --ignore-scripts build
npm --prefix packages/qa-contracts run --ignore-scripts test
```

Run the Rust conformance checks through the Local QA workspace:

```bash
cargo fmt --manifest-path apps/local-qa-runtime/Cargo.toml --all -- --check
cargo clippy --manifest-path apps/local-qa-runtime/Cargo.toml \
  --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path apps/local-qa-runtime/Cargo.toml \
  --workspace --locked -- --nocapture
```

These libraries provide contract validation and deterministic bytes only. They
do not start a process, expose an endpoint, authorize a request, persist state,
verify signatures, invoke a worker, or perform any other side effect.
