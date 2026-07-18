# workflow-writer

`workflow-writer` is a **meta-adapter** on the shared `workflow.engine.*` kernel. It lets a
user **author a brand-new `fkst.workflow.v1` template** or, critically, **refine an existing
workflow implementation package's template** — for example change `workflow-security`'s
`security-review` steps, or tweak a `workflow-dev` template — by filing a single labelled
request issue. The change is delivered as a **reviewable pull request**, and the drafted
template is gated by the **kernel's** `blueprint.validate` (reused, never re-implemented). It
copies **no** engine logic — the branching / idempotency / frontier / marker / CAS-key
machinery lives once in the kernel and is reached through `bindings.lua`.

## How a user files a request

Open an issue with the **`fkst-workflow`** label describing the workflow you want. Two modes
are supported:

### Create a new template

> **Title:** Author workflow template: weekly release digest
>
> Draft a workflow that collects merged PRs and drafts a release digest.
> `id: release-digest-flow`

### Refine an existing workflow package

Name the target package (and optionally the workflow id) with `key: value` directives on
their own lines. Only packages on the refinable allowlist may be edited in place:

> **Title:** Refine workflow template: add a licence-scan step
>
> Please **refine** the security review to add a licence-scan step before filing findings.
> `target: workflow-security`
> `workflow-id: security-review`

The agent reads the target package's existing template, applies **only** the requested
change, keeps the template **id unchanged** (an in-place edit), self-validates, and opens a
PR that edits the target file. A refine request that names a package outside the allowlist
(`workflow-security`, `workflow-finance`, `workflow-marketing`, `workflow-dev`,
`github-devloop-workflow`) is conservatively downgraded to a *create*, so untrusted issue
text can never steer an edit into an arbitrary path.

## What it does (single degenerate step)

The built-in `workflow-authoring-flow` template has ONE generated step, `author-template`:
the injected codex runner drafts (or refines) the template, echoes the strict-JSON template
on stdout, writes it under `FKST_WORKFLOW_CATALOG_ROOT` (create) or edits the target file
(refine), and opens a PR that closes the request. Completion is the **PR merging** — so a
merged PR advances the frontier to terminal-done.

- **Executor** (`executor.lua`) — spawns codex, then **gates the result on the kernel
  validator** (`authoring.validate_drafted_template` → `workflow.engine.blueprint`) and the
  **id-collision guard** before recording success. A draft that fails validation or collides
  with an existing catalog id is refused (`fail`), so an untrusted request can never land an
  invalid template. Idempotent on `materialization.child_dedup_key`: a re-reconcile finds the
  existing PR marker and returns `exists` (never a second PR); an in-flight create returns
  `wait`.
- **Completion** (`completion.lua`) — a fresh, pure PR-state reader: merged →
  `result_ready`, open → `running`, transient read → `recoverable`, closed-unmerged / invalid
  draft → `fatal`, unreadable → `unknown`.
- **Catalog** (`catalog.lua`) — merges the built-in record with `FKST_WORKFLOW_CATALOG_ROOT`
  files through the one validator; also exposes `catalog_ids()` for the collision guard.

## It claims its OWN work (never the dev intake seam)

`workflow-writer` is **not** part of the development topology. It never consumes
`github-devloop-intake.devloop_intake_candidate` (the single-consumer INTAKE_POLICY_SET
invariant stays intact). It claims work through its own path only:

- the `fkst-workflow` **label** on a request issue (discovered during polling), and
- its own cron tick `raisers/writer_poll.lua` (`workflow_writer_tick`) + its own
  `workflow_authoring_request` queue.

Its marker namespace is `fkst:workflow-writer`, so its issue markers never collide with a
co-resident adapter's.

## Untrusted-content safety

The request issue text is only ever **data**: it is byte-clamped before it reaches the codex
prompt, and any drafted template is only ever DATA through the byte-bounded kernel validator
(id ≤ 128, version ≤ 64, summary ≤ 512, applies_when ≤ 1024, static intent / generator ≤
8000, 1..16 contiguous steps, unknown-field rejection). Delivery is a **reviewable PR** (a
human gate); an id colliding with a shipped template surfaces as a refusal and, on load,
would disqualify both peers — so a draft can never silently override a shipped flow.

## Files

| File | Role |
|------|------|
| `fkst.toml` | `lib_deps=[contract, workflow, testkit, forge, devloop]`, `event_deps=[github-proxy]`; no dev intake. |
| `bindings.lua` | Composes the four kernel seams + own intake; the only GitHub/runner touch-point. |
| `authoring.lua` | Refine-vs-create routing, the kernel-validator-reuse gate, id-collision guard, prompt composition, constants. |
| `records.lua` | The built-in `workflow-authoring-flow` template + `records()` / `builtin_ids()`. |
| `completion.lua` | Pure PR-state child-status reader (maps the PR lifecycle to the 5-status enum). |
| `discovery.lua` | `platform.discovery`/`lease`: labelled-issue reads + PR-state resolution + marker parsing (own namespace). |
| `executor.lua` | `executor.raise_step`/`emit_terminal`: spawns codex, validates the draft, opens/records the PR marker. |
| `catalog.lua` | Blueprint provider (built-in + `FKST_WORKFLOW_CATALOG_ROOT`, one validator) + `catalog_ids()`. |
| `intake.lua` | Own intake handlers (label + own queue), stamps the blueprint, drives the tick. |
| `blueprints/authoring-flow.json` | On-disk equivalent of the built-in template (catalog-lint validated). |
| `prompts/author_template.lua`, `prompts/refine_template.lua` | Base codex instruction text (data) for each mode. |
| `departments/*/main.lua` | Thin saga-shaped wrappers over the kernel handlers. |

## Template id-collision warning

Host files under `FKST_WORKFLOW_CATALOG_ROOT` load through the same `blueprint.validate`. A
host-authored template whose `id` collides with a shipped template id **silently disqualifies
BOTH** peers (the kernel's duplicate-id rule). Choose a distinct id for a *create*; keep the
id **unchanged** for a *refine*.

## Tests

```sh
scripts/run.sh test workflow-writer
```

- `core_test` — drafted-template validation (kernel validator reuse), refine-vs-create
  routing, id-collision guard, prompt composition, PR-state completion mapping, built-in
  blueprint validity.
- `catalog_lint_test` — the on-disk `blueprints/authoring-flow.json` validates under the
  kernel validator and matches the built-in identity.
- `namespaced_dispatch_test` — every department accepts its consumed queues under their
  production namespaced names, and the select department **rejects** the dev intake candidate
  seam.
- `fire_raiser_writer_poll_test` — producer-liveness: the `writer_poll` cron tick routes to
  `workflow_writer_select` (trace asserts consumer_result/source_payload/raised/routed_to).
- `run_graph_writer_authoring_smoke_test` — the materialization tick reaches the materializer
  through the real composed graph and quiesces cleanly.
