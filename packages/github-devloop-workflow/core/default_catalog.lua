local M = {}

-- Built-in catalogs are authored in the SAME JSON shape a host provides an
-- external catalog under FKST_WORKFLOW_CATALOG_ROOT/*.json -- ONE writing style
-- for host and non-host. Embedded as JSON strings (not bundled files) because
-- core cannot locate its own package root at runtime (debug is forbidden, file
-- is runtime-relative). Each entry decodes LAZILY via json.decode in records();
-- decoded records flow through the SAME catalog.validate_records as file
-- catalogs -- no "trusted because ours" bypass.
--
-- The built-in COUNT is structural (known without decoding), so module load and
-- the engine graph_scan pure-primitives spec-eval context -- which has no json
-- -- never force a decode. json.decode happens only at real pipeline runtime.
local SOURCES = {
  {
    path = "builtin:software-feature-flow",
    json = [==[
{
  "schema": "fkst.workflow.v1",
  "id": "software-feature-flow",
  "version": "2026-07-05",
  "summary": "Deliver a new bounded software capability as two independently mergeable vertical increments: walking skeleton, then production slice.",
  "applies_when": "Conservative router: choose this only when the origin issue text unambiguously matches this flow and exactly one flow: it asks to implement a NEW software capability or feature deliverable as one bounded slice with a real end-to-end path. Do not choose it for epics, trackers, broad multi-slice features, trivial one-PR changes, bug fixes, refactors, contract migrations, docs-only work, spikes, or ambiguous requests; choose none/plain devloop instead.",
  "steps": [
    {
      "id": "walking-skeleton",
      "title": "Build the walking skeleton",
      "content": {
        "kind": "generated",
        "generator": "Read the origin issue from its source_ref and emit exactly one child issue title and body for the walking-skeleton step. Scope the child PR to the thinnest executable end-to-end path for one tiny accepted behavior, wiring the major components through real code. It must compile/run and be green with a smoke or acceptance test that proves the architecture. This is a Cockburn walking skeleton, not an empty scaffold or placeholder. TDD/tests live inside this child PR: the child writes its own tests and leaves them green. Do not ask the child to duplicate devloop consensus, CI, PR review, or merge gates; devloop provides those. If no architectural seam or mergeable diff exists, the child must produce no changes with a clear WHY; no-changes is fatal and blocks the origin."
      }
    },
    {
      "id": "production-slice",
      "title": "Complete the production slice",
      "content": {
        "kind": "generated",
        "generator": "Read the origin issue and the MERGED walking-skeleton result through the predecessor source_ref, then emit exactly one child issue title and body for the production-slice step. Scope the child PR to complete the accepted behavior on top of the merged skeleton, including edge cases, negative cases, and tests, all green. Keep it one bounded vertical increment. TDD/tests live inside this child PR: the child writes its own tests and leaves them green. Do not ask the child to duplicate devloop consensus, CI, PR review, or merge gates; devloop provides those. If no production-slice diff remains, the child must produce no changes with a clear WHY; no-changes is fatal and blocks the origin."
      }
    }
  ]
}
]==],
  },
  {
    path = "builtin:software-refactor-flow",
    json = [==[
{
  "schema": "fkst.workflow.v1",
  "id": "software-refactor-flow",
  "version": "2026-07-05",
  "summary": "Restructure existing code without observable behavior change using characterization tests, then behavior-preserving refactoring.",
  "applies_when": "Conservative router: choose this only when the origin issue text unambiguously matches this flow and exactly one flow: it asks to RESTRUCTURE or clean up existing code WITHOUT changing externally observable behavior. Do not choose it for new features, bug fixes, contract migrations, API/schema/CLI/event/persisted-format changes, broad epics, or ambiguous requests; choose none/plain devloop instead.",
  "steps": [
    {
      "id": "characterization-tests",
      "title": "Add characterization tests",
      "content": {
        "kind": "generated",
        "generator": "Read the origin issue from its source_ref and emit exactly one child issue title and body for the characterization-tests step. Scope the child PR to tests only: add Feathers-style characterization tests that pin the CURRENT externally observable behavior of the target code and pass against the current code before any restructuring. Do not change production behavior. TDD/tests live inside this child PR: the child writes its own tests and leaves them green. Do not ask the child to duplicate devloop consensus, CI, PR review, or merge gates; devloop provides those. If the current behavior is already fully pinned or no target seam exists, the child must produce no changes with a clear WHY; no-changes is fatal and blocks the origin."
      }
    },
    {
      "id": "behavior-preserving-restructure",
      "title": "Restructure without behavior change",
      "content": {
        "kind": "generated",
        "generator": "Read the origin issue and the MERGED characterization-tests result through the predecessor source_ref, then emit exactly one child issue title and body for the behavior-preserving-restructure step. Scope the child PR to Fowler-style internal restructuring only, keeping the characterization tests and existing tests green. Preserve externally observable behavior exactly; do not add feature behavior, bug-fix semantics, or contract changes. TDD/tests live inside this child PR: any test maintenance required by the restructure stays in the same green child PR. Do not ask the child to duplicate devloop consensus, CI, PR review, or merge gates; devloop provides those. If no behavior-preserving restructure diff remains, the child must produce no changes with a clear WHY; no-changes is fatal and blocks the origin."
      }
    }
  ]
}
]==],
  },
  {
    path = "builtin:software-contract-migration-flow",
    json = [==[
{
  "schema": "fkst.workflow.v1",
  "id": "software-contract-migration-flow",
  "version": "2026-07-05",
  "summary": "Change an existing software contract through expand, migrate, and contract increments with old and new forms coexisting temporarily.",
  "applies_when": "Conservative router: choose this only when the origin issue text unambiguously matches this flow and exactly one flow: it names an EXISTING API, schema, event payload, CLI surface, adapter seam, or persisted format to change, replace, deprecate, or remove, old plus new forms must coexist temporarily, AND the repository itself owns the producers or consumers (in-repo call sites) that the migrate step will move to the new form. Do not choose it for simple one-PR changes, new features without contract replacement, pure refactors with no contract change, docs-only work, bug fixes, purely external or published-contract migrations with no in-repo consumers to move, or ambiguous requests; choose none/plain devloop instead.",
  "steps": [
    {
      "id": "expand",
      "title": "Expand the contract",
      "content": {
        "kind": "generated",
        "generator": "Read the origin issue from its source_ref and emit exactly one child issue title and body for the expand step. Scope the child PR to Fowler Parallel Change expand: add the new contract path plus a backward-compatible adapter, bridge, or dual-read-or-write path, with executable contract tests green. The old contract must still work. TDD/tests live inside this child PR: the child writes its own tests and leaves them green. Do not ask the child to duplicate devloop consensus, CI, PR review, or merge gates; devloop provides those. If the named existing contract or coexistence seam does not exist, the child must produce no changes with a clear WHY; no-changes is fatal and blocks the origin."
      }
    },
    {
      "id": "migrate",
      "title": "Migrate producers and consumers",
      "content": {
        "kind": "generated",
        "generator": "Read the origin issue and the MERGED expand result through the predecessor source_ref, then emit exactly one child issue title and body for the migrate step. Scope the child PR to move in-repo producers and consumers to the new contract form on top of the merged backward-compatible bridge. Keep old and new forms working during this step and keep tests green. TDD/tests live inside this child PR: the child writes or updates its own tests and leaves them green. Do not ask the child to duplicate devloop consensus, CI, PR review, or merge gates; devloop provides those. If no in-repo producer or consumer migration diff exists, the child must produce no changes with a clear WHY; no-changes is fatal and blocks the origin."
      }
    },
    {
      "id": "contract",
      "title": "Contract the old form",
      "content": {
        "kind": "generated",
        "generator": "Read the origin issue and the MERGED migrate result through the predecessor source_ref, then emit exactly one child issue title and body for the contract step. Scope the child PR to remove the old contract form and the temporary bridge/adapter/deprecation path, leaving the new form as the sole in-repo path with tests green. TDD/tests live inside this child PR: the child writes or updates its own tests and leaves them green. Do not ask the child to duplicate devloop consensus, CI, PR review, or merge gates; devloop provides those. If the old form and bridge are already gone, the child must produce no changes with a clear WHY; no-changes is fatal and blocks the origin."
      }
    }
  ]
}
]==],
  },
}

-- Structural count of built-in catalogs: no json.decode, safe at module load.
M.count = #SOURCES

-- Raw records (path + decoded blueprint), NOT validated here -- validation is
-- singular in catalog.validate_records over both built-in and file records.
function M.records()
  local out = {}
  for _, src in ipairs(SOURCES) do
    local ok, decoded = pcall(json.decode, src.json)
    if ok then
      out[#out + 1] = { path = src.path, blueprint = decoded }
    else
      out[#out + 1] = { path = src.path }
    end
  end
  return out
end

function M.install(target)
  target.default_catalog = M
end

return M
