# frontend-devloop

`frontend-devloop` is a composed profile package for host repositories whose primary product is a UI application. It owns the declarative profile contract for running the existing GitHub devloop package family against a frontend host; it does not own browser automation, GitHub issue lifecycle state, or host package-manager commands.

The profile exists because project-local scripts alone can run `install`, `lint`, `test`, and `build`, but they do not tell the fkst host-run contract which platform packages and trust boundaries make a UI workflow safe to supervise. `browser-qa` remains the owner of browser execution and visual validation. `github-devloop` remains the issue-to-PR lifecycle owner.

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
