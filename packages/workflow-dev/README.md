# workflow-dev

`workflow-dev` is the **development adapter** topology, shipped as a **thin composed
profile package**. It owns the declarative profile that runs the existing GitHub devloop
package family as a development host and **selects the workflow-engine intake policy**
(`github-devloop-workflow`, the `workflow` topology) instead of the default topology.

It owns **no engine or business logic**:

- The development engine — the proven issue → child workflow → code-atom lifecycle — is the
  existing `github-devloop-workflow` package. `workflow-dev` does **not** rename, gut, or
  re-implement it.
- The generic workflow machinery (blueprint / catalog / frontier / marker / materialization /
  reconcile) lives once in the shared `workflow.engine.*` library kernel. `workflow-dev`
  copies none of it — the zero-tolerance cross-file dedup ratchet stays satisfied.

## Why a profile package (and not host-local scripts)

Project-local scripts alone can run `install`, `lint`, `test`, and `build`, but they cannot
declare which platform packages and trust boundaries make a **workflow-engine development
host** safe to supervise. The necessity proof is part of `workflow-dev.profile.v1`, not an
implied convention:

- The **default-topology profile** (`frontend-devloop` + `github-devloop-intake-default`) can
  express reusable devloop composition and the default intake policy, but it selects
  `github-devloop-intake-default` — not the workflow-engine intake policy — so it cannot own
  the workflow development profile.
- The **`github-devloop-workflow` package** owns the workflow-engine intake seat and
  materialization lifecycle, but it does not own which platform packages a development host
  composes or the host trust boundaries — putting that inside the engine would couple
  materialization to host composition.

Therefore `workflow-dev` owns only the development workflow profile contract that composes
those existing surfaces.

## The single intake-policy seat

The intake-policy slot (`scripts/intake_policy_slots.json`) has exactly two implementations:
`github-devloop-intake-default` (default topology) and `github-devloop-workflow` (workflow
topology). A `workflow-dev` host composes **only** `github-devloop-workflow`, so the
`github-devloop-intake.devloop_intake_candidate` seam keeps a single consumer.
`workflow-dev` itself never consumes that seam — it only composes the package that does.

## Host package composition

A workflow development host includes these platform package roots in
`.fkst/compose/package-roots`:

```text
fkst-packages:packages/github-proxy
fkst-packages:packages/consensus
fkst-packages:packages/github-devloop-intake
fkst-packages:packages/github-devloop-workflow
fkst-packages:packages/github-devloop-decompose
fkst-packages:packages/github-devloop
fkst-packages:packages/github-devloop-pr
fkst-packages:packages/github-devloop-ops
fkst-packages:packages/github-devloop-integration
fkst-packages:packages/workflow-dev
```

## Handoff and dead-letter

The `workflow-dev.handoff.v1` handoff is source-ref only: code artifacts, worktrees, and
codex results stay in the host worktree and are referenced by `source_ref`; they are never
serialized into reliable delivery payloads. A `dead_letter` department drains the reliable
`dead_letter` queue under the `workflow-dev` tag.

## Tests

```sh
scripts/run.sh test workflow-dev
```
