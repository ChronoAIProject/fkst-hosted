# Refactor SPEC: collapse the GitHub-egress capability to one owner (kill the "wire-it-into-every-consumer" trick class)

Status: proposed (sshx 5-seat philosopher panel + ChatGPT Pro oracle, 6/6 converged, 2026-07-09)
Scope owner: fkst-packages (+ a named fkst-substrate egress-capability follow-up)
Motivation: PR #2049 (`docs/superpowers/specs/2026-07-09-gh-ingress-content-whitelist.md`) landed a one-concept
security change — "redact non-whitelisted GitHub authors' content at the gh boundary" — that had to edit ~144 files.
That blast radius is a mechanism smell, not verbosity. This SPEC roots the mechanism and prescribes the refactor that
makes the next cross-cutting GitHub-egress change touch ~3 files, not ~144.

## 1. Root cause (why one cross-cutting change touched ~144 files)

**Ownership inversion: a cross-cutting egress invariant is owned by many consumers instead of one producer.**
The invariant "every GitHub read redacts non-whitelisted authors' content" is not owned by the GitHub boundary — the
repo represents GitHub access as something many consumers can **construct, wrap, mock, or invoke**. Once the raw
primitive is reachable, every caller joins the security perimeter, so a one-line policy change became a many-file edit.

The repo's own doctrine already prescribes the cure and this violated it: **capability restriction (business code
cannot reach the raw primitive) > runtime guard > declarative schema > per-case scan; a scan is a migration backstop,
not the seam** (CLAUDE.md "Harness essence"). Here a literal-`gh` ratchet was treated as the seam; the obfuscated
`table.concat({"g","h"})` head in `libraries/devloop/gh_exec.lua` proves the scan was policing *text*, not
*capabilities*.

**Corrected magnitude (seek-truth-from-facts):** the change is **144 files** (+3056/−886): approximately **72
non-test production/wiring** + **69 test** files. A strict construction/factory-seam grep finds 33 lines across 23
production files; a broad construction/egress-surface grep finds 95 lines across 41 files. (Earlier informal
"~200 / ~78 / ~82" figures were overstated.)

## 2. The trick class (audited across the repo — each is a face of the same root cause)

| # | Trick | Evidence (file:line) | Natural owner | Made unrepresentable by |
|---|-------|----------------------|---------------|--------------------------|
| T3 | Second gh egress + bypass-by-obfuscation | `libraries/devloop/gh_exec.lua:6,37` (`table.concat("g","h")`) | forge.github egress adapter | delete it; raw gh argv exec private to forge.github; business code cannot name gh authority |
| T2 | Handle construction forced onto ~23–41 consumers | `libraries/forge/ports.lua:48`, `libraries/devloop/github_factory.lua:19`, `packages/github-devloop/core.lua:81`, `packages/github-devloop-pr/core.lua:87`, `libraries/devloop/github_proxy_entity_view.lua:27`, direct `forge.github.new` | one production GitHub capability factory | consumers receive an already-authorized handle; no public way to build an unsafe one |
| T4 | Duplicated / divergent enforcement | two filters (`forge.github.content_filter` + `libraries/devloop/content_provenance.lua`), two `[bot]`-canon functions, duplicated bot-identity parsing `libraries/devloop/claims.lua:95`, duplicated author-policy producers `libraries/forge/ports.lua:12` + `libraries/devloop/github_author_policy.lua:17` + `libraries/devloop/github_factory.lua:8` | one AuthorPolicy provider captured once by the handle | single mechanism + single policy value; aliases delete after callers drain |
| T1 | Scan-as-primary-defense | `scripts/check_repo_github_content_ingress.py:91,171,197`; G-ADAPTER `check_repo_gh_git_adapter.py` | the capability boundary, not a text scanner | typed read APIs require the content policy internally; scan shrinks to a migration backstop, then deletes |
| T7 | Multiple non-composing test scaffolds | `libraries/testkit/devloop_fixtures.lua:331` (`mock_author_policy_env`) vs `mock_context_bundle` (which clobbered it) + a bot-aware runner + ~12 local helper defs | one testkit fake GitHub port + policy fixture | tests inject one fake `ports.github` with explicit policy; no ambient whitelist env, no parallel mock systems |
| T5 | Proxy-over-truth patches (general) | `migration/monotone-gate.allowlist`; marker-age / state proxies | the lifecycle/state producer | typed accessors (`reached()` / `holds()`) as the only API; the raw text gate becomes unrepresentable |

**Generalization — the SAME pattern, OUT OF THIS refactor's scope (future slices):** marker trust forced onto
consumers (`libraries/devloop/parsers/misc.lua:105`, `libraries/devloop/markers/facts.lua`,
`packages/github-proxy/core/marker_guard.lua`) → a `TrustedMarkerStream` capability owned by the marker-fact producer;
dedup / source_ref semantics leaked to consumers (`packages/github-proxy/core.lua:214`, `libraries/devloop/entity.lua`,
`libraries/devloop/merge_queue.lua`) → typed reliable-event constructors that own dedup lineage.

## 3. Target architecture

**Centralize AUTHORITY + mandatory redaction; decentralize domain use of already-safe data** — do NOT build a
god-module for all GitHub semantics.

```
business code (owns domain decisions over REDACTED results)
  -> injected GitHub capability (typed ops: get_pr / list_comments / read_issue ...; outputs already redacted)
    -> ONE Lua gateway / composition seam (mints the capability, binds AuthorPolicy once)
      -> fkst-substrate GitHub egress (owns raw gh/process/network; mandatory redaction at the boundary)
```

- **One seam mints GitHub authority.** The returned object is not a raw command runner — typed methods, outputs
  already redacted per the whitelist. Consumers never pass author policy per call, never build gh argv, never read
  whitelist env.
- **The engine (fkst-substrate) enforces the hard boundary** (capability restriction): business/package Lua cannot
  execute raw `gh` / arbitrary process egress that is semantically GitHub egress. This is the named substrate
  follow-up; until it lands, the Lua gateway + shrink ratchet hold the line.
- **AuthorPolicy is data supplied at the composition root** (devloop owns policy-from-env; forge owns transport +
  filter mechanism). One provider, captured once by the handle.

## 4. Shrink-only, inventory-to-zero ratchet ladder (no big-bang; never rewrites a live state machine)

Hard scope freeze: **GitHub authored-content egress + its tests only.** No restart table, marker grammar, queue
topology, intake trust, PR-diff trust, or substrate change beyond the egress capability is bundled into this refactor.
Every step must **monotonically shrink** the count of constructors / mock systems / ambient-policy channels.

- **Step 0 — Inventory (three shrink-only ledgers).** `migration/gh-egress.inventory` (raw gh exec wrappers),
  `migration/gh-handle-construction.inventory` (every `forge.github.new` / factory / `production_handle` site),
  `migration/gh-authorpolicy-fixture.inventory` (per-test author-policy mocks). CI: new entries fail; removals are the
  only normal movement.
- **Step 1 — Introduce the canonical capability, behavior-preserving.** One production GitHub provider/factory that
  alone receives `exec_argv` + one injected `AuthorPolicy`, exposing typed methods; raw `_exec` private. Existing
  paths *delegate* to it. No behavior change; no state machine rewrite.
- **Step 2 — Ratchet new code.** No new raw gh, no new handle constructor, no new whitelist-env read, no new test mock
  system (extend `G-GITHUB-CONTENT-INGRESS` / G-ADAPTER to enforce).
- **Step 3 — Convert one owner slice at a time.** Per package/library: replace constructor ownership with the injected
  capability; delete its local policy wiring + its bespoke test mock. Each slice reduces surface.
- **Step 4 — Delete shims at inventory zero.** `devloop.gh_exec` (delete the obfuscated head), `devloop.github_factory`
  (fold into the one provider), the `content_provenance` re-export, decentralized `production_handle` / `new` sites —
  deleted only when their ledger reaches zero.
- **Step 5 — Move enforcement into substrate** (follow-up fkst-substrate PR): the engine rejects unauthorized GitHub /
  process egress and returns only redacted GitHub reads. Then the Lua scans are no longer security-critical.
- **Step 6 — Delete scan-as-primary-defense.** Once raw capability is structurally unreachable, shrink
  `G-GITHUB-CONTENT-INGRESS` / G-ADAPTER to adapter-contract-only, or delete.

## 5. Where the "one place" instinct is right — and its limit

Right about **authority construction + egress enforcement**: one place mints GitHub authority, one path to raw egress,
one mandatory redaction at the boundary. Cross-cutting egress policy belongs at the producer seam, not in ~72
consumers. Limit: "one place" must NOT become one god-module for all GitHub *semantics* — do not centralize merge
rules, package workflows, or business interpretation of redacted data. The engine owns raw egress; the Lua gateway
owns capability shape; packages own business decisions over redacted results; tests own scenario data via one
canonical fake. This preserves natural ownership while removing the trick class.

## 6. Success criteria

- The three inventories exist and only shrink; CI fails on growth.
- After migration: exactly one gh egress; one production GitHub construction seam; business code cannot obtain
  `exec_argv`-for-gh or an unauthorized handle; one testkit GitHub fake + policy fixture (the >= 3 colliding mocks gone).
- The gh-content ratchets are demoted from primary defense to migration backstop (or deleted after substrate enforces).
- No live state machine rewritten; each PR is behavior-preserving and monotonically shrinks surface.

⟦AI:FKST⟧
