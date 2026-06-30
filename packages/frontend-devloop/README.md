# frontend-devloop

`frontend-devloop` is a composed profile package for host repositories whose primary product is a UI application. It owns the declarative profile contract for running the existing GitHub devloop package family against a frontend host; it does not own browser automation, GitHub issue lifecycle state, global host hydration, or host package-manager commands.

The profile exists because project-local scripts alone can run `install`, `lint`, `test`, and `build`, but they do not tell the fkst host-run contract which platform packages and trust boundaries make a UI workflow safe to supervise. `browser-qa` remains the owner of browser execution and visual validation. `github-devloop` remains the issue-to-PR lifecycle owner.

The necessity proof is part of `frontend-devloop.profile.v1`, not an implied convention:

- Project-local scripts and `.fkst/compose/package-roots` can express host-owned command execution
  and host-local package root selection, but they cannot own a reusable platform package
  composition, UI workflow trust-boundary declaration, or package-local conformance contract without
  making every frontend host duplicate fkst-packages platform semantics. `.fkst/compose/package-roots`
  remains host input for selected roots; it is not the package-owned authority for UI workflow trust
  boundaries or source-ref-only artifact handoff.
- `browser-qa` can express browser execution and visual validation, but it cannot own reusable
  platform package composition or the GitHub devloop lifecycle without coupling browser execution to
  issue-to-PR orchestration. It validates UI runtime behavior; it does not own the package graph.
- Global-host profiles can express generic host hydration and workspace-root wiring, but they cannot
  own UI workflow trust-boundary policy or source-ref-only UI artifact handoff without coupling the
  generic host layer to frontend workflow semantics. They intentionally exclude package roots and
  frontend workflow semantics.

Therefore `frontend-devloop` owns only the UI workflow profile contract that composes those existing surfaces.

Host package composition is explicit. A frontend host includes these platform package roots in `.fkst/compose/package-roots`:

```text
fkst-packages:packages/github-proxy
fkst-packages:packages/consensus
fkst-packages:packages/github-devloop-intake
fkst-packages:packages/github-devloop-intake-default
fkst-packages:packages/github-devloop-decompose
fkst-packages:packages/github-devloop
fkst-packages:packages/github-devloop-pr
fkst-packages:packages/github-devloop-ops
fkst-packages:packages/github-devloop-integration
fkst-packages:packages/frontend-devloop
```

The `frontend-devloop.profile.v1` handoff is source-ref only. UI artifacts, screenshots, traces, and browser results stay in the host worktree or a browser-QA source and are referenced by `source_ref`; they are not serialized into reliable delivery payloads.

Run the package tests with:

```sh
scripts/run.sh test frontend-devloop
```
