# ADR 0002: Host `.fkst/` layout normalization

- Status: Accepted
- Date: 2026-06-25

---

## Context

Before this ADR, host repositories carried the same underlying facts in several places:

- `.fkst/local-packages/<pkg>/` holds host-owned packages.
- `.fkst/std/` was the host-owned workspace-library directory -> `.fkst/local-libraries/<lib>/`.
- `.fkst/conformance/package-roots` was the composed graph root list -> `.fkst/compose/package-roots`.
- `.fkst/conformance/allowlists/` holds host conformance allowlists.
- `.fkst/run/fkst-packages-conformance/` was the hydrated platform source checkout -> `.fkst/run/fkst-packages-platform/`.
- `.fkst-packages-ref` was the top-level fkst-packages host pin -> `fkst.workspace.toml` `[[external_sources]]` plus `fkst.lock`.

That drift creates four defects:

1. The hydrated checkout name says `conformance` even though it is the pinned platform source.
2. `conformance/package-roots` leaks grouping instead of naming the host composition role.
3. The same host-layout fact is repeated across dirs, source ids, prefix strings, scripts, and docs.
4. Host-layout truth is scattered instead of anchored once.

## Decision

This ADR is the canonical source of truth for the host `.fkst/` structure, including both layout names and the host platform pin mechanism.

Adopt the canonical host layout:

- `.fkst/local-packages/<pkg>/` for tracked host-owned packages.
- `.fkst/local-libraries/<lib>/` for tracked host-owned workspace libraries.
- `.fkst/compose/package-roots` for tracked host composition roots.
- `.fkst/conformance/allowlists/` for tracked host conformance allowlists.
- `.fkst/run/fkst-packages-platform/` for ignored hydrated platform source.

The canonical platform identity is `fkst-packages-platform`.

## Pin mechanism

The canonical host platform pin is `fkst.workspace.toml` `[[external_sources]]` plus `fkst.lock`.
Hosts do not use `.fkst-packages-ref` as the platform pin.

Keep the pinned version separate from the composition list. Do not collapse package roots into workspace pinning or lock state.

## Consequences

- Host repos get one source of truth for layout naming.
- Behavior does not change; this is normalization only.
- The runner and docs now speak the same canonical names.

## Consumer map

- `fkst-packages/scripts/host_entry.sh`: read `.fkst/compose/package-roots`; keep `.fkst/conformance/allowlists/` and `.fkst/local-packages/`.
- `fkst-website/scripts/run.sh`: hydrate `.fkst/run/fkst-packages-platform/`.
- `fkst-website/.github/workflows/*`: hydrate `fkst-packages-platform`.
- `fkst-website/.gitignore`: keep the ignored hydrated checkout comment in sync.
- `fkst-website` docs: point to this ADR and canonical names.

⟦AI:FKST⟧
