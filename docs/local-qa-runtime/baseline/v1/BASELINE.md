# Local QA Runtime v1 Specification Baseline

This directory is the immutable, reviewed specification baseline for Local QA
Runtime v1. It applies configuration-management baselining and requirements
traceability to the approved source snapshot. It describes target behavior; it
does not implement any Runtime, contract, persistence, VM, browser, packaging,
or integration capability.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** in this manifest are to be interpreted as described in RFC 2119 and
RFC 8174 when, and only when, they appear in all capitals.

## Baseline Identity and Provenance

- Baseline identifier:
  `local-qa-runtime-baseline-v1-c1aca6cdf519abde3823d05603d9ca9bb31370de`
- Source repository: `wanghuan-520/workflow-qa`
- Source commit: `c1aca6cdf519abde3823d05603d9ca9bb31370de`
- Approval record: `ChronoAIProject/fkst-hosted#5617`, approved by repository
  maintainer `wanghuan-520` on 2026-07-29.

That human approval covers the source snapshot and digest set, the repository
mapping, the independent version and supersession policy, and the platform and
architecture decision recorded below. This manifest is the sole approved
decision and clarification layer for this baseline.

The imported documents are byte-identical Git blobs from the source commit.
Their approved integrity set is:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `SPEC.zh-CN.md` | 358088 | `d70f6abbc9df84dc2c70684aec7741f0caf2a6aeae43b33f9443c771acca6268` |
| `DESIGN.zh-CN.md` | 106796 | `f515c6d643f285de4c4c0b7a6155207d5f9755149322b33a82e25622fcacb76a` |
| `LOCAL-QA-RUNTIME-DESIGN.zh-CN.md` | 168522 | `e03e85a209f9f3ba3e0f9eca5afe2203e4ef9f0f1a3b7b4fa19a264de05bb2b8` |

The three imported documents MUST NOT be edited, reformatted, translated, or
normalized in place. This manifest MUST NOT duplicate or redefine their wire
contracts. Any future clarification, editorial correction, or normative change
requires human approval and an explicit superseding baseline with a new
identifier, provenance record, and digest set. This baseline remains available
unchanged for historical verification.

## Authority and Conflict Resolution

When the imported documents overlap or appear to conflict, apply this order:

1. `SPEC.zh-CN.md` is normative for cross-boundary fields, exact schemas,
   strict unions, signatures, canonicalization, digests, interfaces, state
   machines, compatibility, errors, and verification requirements.
2. `DESIGN.zh-CN.md` is normative for system responsibilities, authorization
   authorities, trust boundaries, the complete QA Run lifecycle, and
   Workspace, Sandbox, and Environment semantics.
3. `LOCAL-QA-RUNTIME-DESIGN.zh-CN.md` is normative for Local QA Runtime v1
   internal topology, module ownership, transaction boundaries, persistence
   semantics, recovery algorithms, and adapter boundaries.
4. Internal table names, column names, pseudocode, and illustrative messages in
   `LOCAL-QA-RUNTIME-DESIGN.zh-CN.md` are not wire-contract definitions and
   cannot override `SPEC.zh-CN.md`.

The narrowest applicable higher-authority requirement prevails. An
implementation question that the documents do not resolve MUST be returned for
human decision; it MUST NOT be resolved by silently creating a field, schema,
enum, method, state, error code, or competing requirement vocabulary.

## Repository Mapping Decision

The existing `backend/` directory is the current physical implementation of
the authority named `apps/hosted-control-plane` in the imported documents. The
logical name does not require a second control-plane implementation or a new
directory in this repository.

This baseline does not move or refactor `backend/`. Any future physical move to
`apps/hosted-control-plane` requires a separate structural issue, review, and
migration plan.

## Version and Supersession Policy

The following version axes are independent and MUST NOT be inferred from one
another:

| Version axis | Meaning | Controlled by |
| --- | --- | --- |
| Specification baseline | Immutable reviewed documentation snapshot and digest set | This baseline identifier and an explicit superseding baseline |
| Contract or schema major | Compatibility boundary for an exact contract or schema | The applicable rules in `SPEC.zh-CN.md` and separately reviewed contract work |
| Runtime release | Version of packaged executable Runtime artifacts | Future Runtime release and update work |
| Root product version | Version of the hosted product | Root `package.json` |
| Scaffold Cargo version | Cargo package metadata for the inert Rust scaffold | `apps/local-qa-runtime/*/Cargo.toml` and separately reviewed release work |

The directory name `v1` identifies this specification baseline; it does not
silently set or advance any other version axis. A contract-major change MUST
NOT replace this baseline. Editorial and normative changes both require an
explicit superseding baseline that records its relationship to this one,
classifies the change, provides new provenance and digests, and receives human
approval. Product, Runtime, contract, or Cargo version changes remain outside
this baseline and require their own scoped work.

## Platform and Architecture Decision

Local QA Runtime v1 is macOS-first and uses Apple
`Virtualization.framework`. An architecture mentioned in an imported design or
contract is not, by itself, a shipped-support claim.

| Architecture | v1 status | Packaging requirement | Acceptance evidence |
| --- | --- | --- | --- |
| `arm64` | REQUIRED for v1 acceptance | Signed and notarized macOS packaging | Verification on real `arm64` Apple hardware using `Virtualization.framework` |
| `x86_64` | Explicitly deferred | Not a v1 release requirement | A separate acceptance decision and real-hardware verification are required before support may be claimed as shipped |

Simulation, cross-compilation, or contract coverage MAY supplement the required
evidence, but it MUST NOT replace the real-hardware acceptance evidence above.

## Target-State Boundary

The imported documents specify target behavior. The existing
`apps/local-qa-runtime/` Rust binaries and TypeScript workers remain an inert,
independently buildable scaffold. This baseline does not make the scaffold a
functional Runtime and does not define executable contracts or fixtures under
`packages/qa-contracts/` or `fixtures/qa/`.

This manifest introduces no protocol field, schema, enum, method, state, or
error code.

No implementation capability or support state may be claimed solely because it
is described by this baseline. Such claims require the applicable implementation
issue, verification gates, and acceptance evidence.

## Mandatory Future Traceability

Every later Local QA Runtime issue and pull request MUST include a completed
traceability block with the following fields. References MUST preserve the
existing section, requirement, milestone, Runtime increment, Definition of
Done, gate, fixture, invariant, and failpoint identifiers verbatim. A field with
no applicable reference MUST say `Not applicable` and give a reason; omission is
not an acceptable substitute.

```markdown
### Local QA Runtime baseline traceability

- Baseline identifier: `local-qa-runtime-baseline-v1-c1aca6cdf519abde3823d05603d9ca9bb31370de`
- Source commit: `c1aca6cdf519abde3823d05603d9ca9bb31370de`
- Digest set:
  - `SPEC.zh-CN.md`: `d70f6abbc9df84dc2c70684aec7741f0caf2a6aeae43b33f9443c771acca6268`
  - `DESIGN.zh-CN.md`: `f515c6d643f285de4c4c0b7a6155207d5f9755149322b33a82e25622fcacb76a`
  - `LOCAL-QA-RUNTIME-DESIGN.zh-CN.md`: `e03e85a209f9f3ba3e0f9eca5afe2203e4ef9f0f1a3b7b4fa19a264de05bb2b8`
- `SPEC.zh-CN.md` sections: `<section references>`
- `SPEC.zh-CN.md` Definition of Done items: `<section 20.4 item numbers>`
- `DESIGN.zh-CN.md` sections and locked decisions: `<references>`
- `LOCAL-QA-RUNTIME-DESIGN.zh-CN.md` sections: `<section references>`
- Runtime increment: `<R0 | R1 | R2 | R3>`
- System milestone: `<M0 | M1 | M2 | M3 | M4 | M5>`
- Verification gates: `<existing gate identifiers>`
- Fixture corpus and case identifiers: `<existing fixture paths and case identifiers>`
- Invariants: `<existing invariant references or identifiers>`
- Failpoints: `<existing failpoint identifiers>`
- Intentionally deferred or excluded requirements: `<references and rationale>`
```

The traceability block links future work to the baseline; it does not authorize
that work to alter this baseline or bypass the authority order, milestone
prerequisites, verification gates, or human-review requirements recorded here.
