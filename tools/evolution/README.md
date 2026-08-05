# FKST Evolution — generator and convergence verifier

The first proof of the FKST Evolution design: one product capability carried all
the way through to a documentation page, a product-operation skill, screenshots,
a demo video and a slide deck — all derived from the same evidence, and all tied
to one manifest.

Specification: `docs/evolution_temp_spec.md` on the `evolution-temp-spec` branch.
Tracking issue: [#5866](https://github.com/ChronoAIProject/fkst-hosted/issues/5866).

## What lives where

| Path | Owner | Contents |
| --- | --- | --- |
| `.fkst/evolution/config.yaml`, `intent/**` | Repository owner | Policy and human product intent. Read, never written by Evolution. |
| `.fkst/evolution/observed/**`, `changes/**`, `manifest.json` | Evolution | The machine's observation of the product, and the convergence record. |
| `.fkst/evolution/docs/`, `skills/`, `journeys/`, `screenshots/`, `slides/` | Evolution | Generated artifacts. |
| `tools/evolution/` | Humans | This generator. **Not** Evolution output — see below. |
| `tools/evolution/out/` | Build | Journey recordings, captions, rendered PDF/MP4. Git-ignored. |

`tools/evolution/artifact-plan.yaml` is generator *input*, which is why it sits
here rather than under the Evolution root: everything under that root is either
human intent or Evolution output, and folding the generator's own configuration
into the output fingerprint it computes would be circular. In the eventual
package composition these roles move to `fkst-packages`.

## Commands

```bash
npm run evolution:test                      # fingerprint vectors + decision logic
npm run evolution -- fingerprint            # all six fingerprints at HEAD
npm run evolution:journeys                  # run journeys, capture screenshots + video
node tools/evolution/src/render-media.ts    # deck PDF, title card, captioned MP4
npm run evolution -- build-manifest --write # assemble manifest.json
npm run evolution -- verify --github        # the section 17.7 convergence decision
```

Set `FKST_EVOLUTION_TIMESTAMP` to a fixed ISO-8601 value when rebuilding a
manifest you expect to be byte-identical; otherwise `updatedAt` alone moves the
manifest projection and therefore the output fingerprint.

## Regenerating from scratch

```bash
npm ci && npm --prefix frontend ci && npm --prefix tools/evolution ci
npm run evolution:journeys
node tools/evolution/src/render-media.ts
git add .fkst/evolution && git commit -m "..."          # artifacts first
FKST_EVOLUTION_TIMESTAMP=<iso> npm run evolution -- build-manifest --write
git add .fkst/evolution/manifest.json && git commit -m "..."
npm run evolution -- verify --github
```

Artifacts are committed **before** the manifest because the manifest hashes the
Git tree, not the working directory: an uncommitted edit must not be able to
produce a manifest describing bytes no reviewer will ever see.

## Verdicts

`verify` exits `0` on `CONVERGED`, `2` on `CONVERGED_PENDING_CONTROL_PLANE`, and
`1` on `NOT_CONVERGED`.

The middle verdict is the honest one for this deployment. Conditions 1, 2, 3, 5
and 6 are fully evaluable today; condition 4 — corroborating each verification
against a check run published by the configured App — is not, because no control
plane exists to publish one. A condition that cannot be evaluated reports
`not-evaluable` and downgrades the verdict; it never counts as a pass.

## Known gaps

These need the control plane and are tracked on #5866:

- No sync issue, sync pull request, or safety-gated merge. `verify` run twice is
  the stand-in for the specification's restart-and-reconcile no-op proof.
- The webhook router dispatches only `installation`, `installation_repositories`
  and `issues`; Evolution also needs `push`, `pull_request` and `release`.
- `validate_branch_name` rejects `@`, so the `@default` sentinel recorded in
  `config.yaml` cannot yet be expressed by the control plane.
- Whether the singleton level-triggered queue can live in packages, or requires
  an `fkst-substrate` change, is unanswered and load-bearing.

## Fingerprint compatibility

`src/hash.ts` documents the exact byte-level serialization, and
`test/hash.test.ts` pins it as vectors. Those expected digests are a wire
contract: changing one is a breaking protocol change, not a failing test to fix.
