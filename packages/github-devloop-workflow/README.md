# github-devloop-workflow

A **workflow-orchestration layer** on top of the stable `issue → consensus → PR → merge`
autonomous-development atom. The package ships built-in software-development workflows, and hosts
may add bounded, multi-step **workflow templates** through `FKST_WORKFLOW_CATALOG_ROOT`. When an
incoming issue matches, the system runs the atom repeatedly — one issue per step, in order — where
each next step's issue content may be **generated from the merged result of the prior step**.

This is a **composed package** (an intake-policy sibling of `github-devloop-intake-default`). It is
additive: it does not modify the atom. Static ("run a fixed N-step plan") is the degenerate case of
the general model, **dynamic result-driven materialization**.

## The three authorities

- **Blueprint** — an immutable origin fact written once when an issue matches a workflow: the
  workflow id, template digest, ordered step slots, per-slot generator contract, and bounds. It holds
  **no rendered issue bodies**.
- **Materialization ledger (CAS)** — the authority for what was actually produced, per slot:
  predecessor-result ref digest, generator-contract digest, generated-spec digest, child dedup key,
  child ref; states `pending → created` in current writes. Two different bodies for one slot are
  unrepresentable. The generated issue title/body is never serialized into an origin marker/comment.
- **Frontier (derived)** — the first slot whose predecessor child is merged and which is not yet
  materialized. Ordering falls out of this; it is computed, never stored.

## How it works

- **`workflow_select`** (intake policy) consumes each intake candidate. On an issue that matches a
  workflow (cheap selector prefilter → a bounded codex judgment), it takes the origin materialization
  lease (assignee) and writes ONE immutable `workflow-blueprint` marker co-located with a `track`
  marker — **no child, no content**. A non-workflow issue is delegated, with zero drift, to the same
  canonical default intake engine `github-devloop-intake-default` uses.
- **`workflow_materialize_next`** (a bounded level-triggered reconciler, 5-minute poll) discovers
  leased blueprint-bearing origins and performs exactly one action per origin: **materialize** the
  frontier slot's child issue (static slot = literal body; generated slot = a codex that fetches the
  predecessor's merged result by `source_ref` and produces the next issue's title/body), **wait** for
  a still-running predecessor, or write a **terminal** marker (`done` / `blocked` / `error`). Each
  materialized child is an ordinary devloop issue whose merged result feeds the next slot.

Replay is idempotent by construction: materialization raises the child create immediately, using a
deterministic child dedup key. Later polls first reconcile an already-created child from the
github-proxy parent `issue-created` marker or child issue body/search before running the generator
or creating anything. The origin ledger remains append-only per-slot marker comments, and
github-proxy's parent intent/created marker comments remain in place; those invisible comments are
functional idempotency facts, not user-facing workflow content.

## Template format

The built-in catalog entries are embedded in Lua and validate through the same catalog validator as
external files:

- `software-feature-flow` — walking skeleton, then production slice for one bounded new capability.
- `software-refactor-flow` — characterization tests, then behavior-preserving restructure.
- `software-contract-migration-flow` — expand, migrate, then contract for an existing contract change.

`workflow_select` is a conservative text router for these flows: it defaults to `none` / plain
devloop unless the origin issue title/body unambiguously matches exactly one flow's `applies_when`. The
first child step is the feasibility gate; no-changes is fatal and blocks the origin with a WHY.

Additional host-authored workflows can be placed under
`FKST_WORKFLOW_CATALOG_ROOT` as `**/*.json` or `**/*.toml` files (one workflow per file). Schema
`fkst.workflow.v1`:

```json
{
  "schema": "fkst.workflow.v1",
  "id": "release-hardening",
  "version": "1",
  "summary": "Harden a release: API surface, then docs generated from the merged API change.",
  "applies_when": "The issue asks to harden or ship a release across API and docs.",
  "selector": {
    "labels_any": ["release"],
    "title_contains_any": ["harden", "release"]
  },
  "steps": [
    {
      "id": "api",
      "title": "Harden the release API surface",
      "content": {
        "kind": "static",
        "intent": "Audit and harden the public API surface for the release: validate inputs, tighten error handling, add the missing tests."
      }
    },
    {
      "id": "docs",
      "title": "Update docs for the hardened API",
      "content": {
        "kind": "generated",
        "generator": "Read the merged PR from the previous step (its diff and the backing issue) and produce a docs-update issue covering exactly the API changes that landed — endpoints, parameters, and error semantics that changed."
      }
    }
  ]
}
```

- **`selector`** (optional) is a cheap prefilter: `labels_any` (any listed label present on the issue)
  and/or `title_contains_any` (any substring in the title). A template with no selector is always
  eligible for the codex judgment. A prefilter match only *offers* the workflow to the codex, which
  makes the final decision (or `none`).
- **`steps`** is an ordered array (linear v1). Each step's `content.kind` is exactly one of:
  - **`static`** — a literal `intent` compiled to a constant generator (no codex): the issue body is
    the intent.
  - **`generated`** — a bounded `generator` instruction: a codex receives the predecessor's result by
    `source_ref` and produces the next issue's spec. **Content is not passed in payloads** — the codex
    fetches full prior context from source.
- Bounds: linear, fixed slot count (`MAX_WORKFLOW_STEPS`). **v1 is content-dynamic, not
  continuation-dynamic** — a step's result cannot add/remove slots (a bounded agent-loop is a
  separate future spec).

Fail-closed validation rejects a template on: invalid JSON, wrong/missing schema, missing/oversized
fields, empty or over-bound steps, duplicate step ids, a `content.kind` that is neither static nor
generated, a static step carrying a `generator`, or a generated step carrying an `intent`. A rejected
template is skipped (logged), not silently half-loaded; a duplicated workflow id disqualifies only the
colliding ids.

## Running it

- **Catalog root**: the built-in default catalog is always loaded. If `FKST_WORKFLOW_CATALOG_ROOT`
  is set, its files are loaded additively and duplicate workflow ids across built-in and external
  sources fail closed. There is no `$HOME/.fkst/workflow` fallback.
- **Topology**: a workflow-enabled topology loads `github-devloop-workflow` as the active intake
  policy. The `INTAKE_POLICY_SET` ratchet allows exactly `github-devloop-intake-default` **or**
  `github-devloop-workflow` to consume the intake-candidate seam per topology (a third consumer is
  CI-red); load one active policy package.
- **Posture**: `FKST_GITHUB_WRITE=1` for real writes; unset = dry-run. This is the only posture switch.

## Markers (bot-authored, trusted)

- `fkst:github-devloop-workflow:blueprint:v1` — origin, immutable structure (workflow id + plan digest).
- `fkst:github-devloop-workflow:materialization:v1` — per-slot CAS ledger (state + digests + child).
- `fkst:github-devloop-workflow:terminal:v1` — origin terminal (`done`/`blocked`/`error` + reason code;
  full prose in the comment body).
- `fkst:github-devloop-workflow:lineage:v1` — in a materialized child's issue body (origin + blueprint
  digest + slot), so the child is recognized as an ordinary workflow-step issue.

The full design rationale (seven-round adversarial convergence) is in
`docs/superpowers/specs/2026-07-02-workflow-orchestration-layer-design.md`. The built-in mature
software workflow catalog is specified in
`docs/superpowers/specs/2026-07-05-mature-builtin-workflows-design.md`.

⟦AI:FKST⟧
