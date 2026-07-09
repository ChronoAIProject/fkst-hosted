local M = {}

-- Built-in catalogs are authored in the same schema shape a host provides under
-- FKST_WORKFLOW_CATALOG_ROOT. Embedded as JSON strings (not bundled files) because
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
  {
    path = "builtin:idea-to-goal-flow",
    json = [==[
{
  "schema": "fkst.workflow.v1",
  "id": "idea-to-goal-flow",
  "version": "2026-07-09",
  "summary": "Converge a fuzzy raw idea, exploration, or open-ended wish whose objective is not yet formed into one concrete, code-verifiable objective derived from its purpose and essence, then implement against that converged goal.",
  "applies_when": "Conservative router: choose this only when origin issue text unambiguously matches this flow and exactly one flow. Discriminator: is the GOAL / OBJECTIVE clear? Choose idea-to-goal-flow ONLY when OBJECTIVE itself is genuinely fuzzy/unformed: open-ended ideas/explorations, a raw idea that must be philosophically converged into ONE concrete objective. POSITIVE: \"we should make the pipeline smarter somehow\"; \"improve the dogfood experience\"; \"maybe rethink how X works\". NEGATIVE -> choose none/plain devloop: \"Login button returns 500 on submit, please fix\" -- OBJECTIVE is clear (stop the 500) though fix location is unknown. Ordinary bug reports, even symptom-only ones, go to plain devloop. Well-specified feature -> software-feature-flow; refactor/migration -> software-refactor-flow or software-contract-migration-flow. If a concrete OBJECTIVE is already clear (even a symptom bug whose fix location is unknown), choose plain devloop or a type-specific flow. When in doubt, choose plain devloop.",
  "steps": [
    {
      "id": "converge-to-goal",
      "title": "Converge the idea into a clear goal and plan",
      "content": {
        "kind": "generated",
        "generator": "Read the origin issue from its source_ref (full body + all comments + linked entities) and investigate the repository at ground-truth to emit exactly one child issue title and body for the converge-to-goal step. Scope the child PR to ADD ONE plan document only -- a new Markdown file under `docs/devloop/plans/` named for the origin issue (e.g. `docs/devloop/plans/<origin-issue-number>-<slug>.md`) -- and change NO production code. If the origin ALREADY has a clear objective/goal, FAIL-SOFT: still produce a minimal plan document that RESTATES the already-clear goal plus its acceptance as a thin but real plan, so a mis-route costs only one extra thin plan PR and slot 2 implements normally. Otherwise, PHILOSOPHICALLY CONVERGE the vague idea into a clear goal: (1) GOAL -- apply teleology (what is this idea FOR -- its purpose and essence) to converge the fuzzy idea into ONE clear, concrete, code-verifiable objective; state it as a single sentence a separate implementer can execute without re-deriving the intent; (2) Grounding -- seek truth from facts: cite source ground-truth (file:line / marker / CI fact / existing behavior) the goal rests on; if the idea is a problem, the located root cause; if it is a new idea, the concrete essence and smallest real end-to-end shape; (3) Approach -- the beautiful solution faithful to the essence: anchored in established prior art / best practice (name it), no magic number, no proxy signal, no symptom-branch, making illegal states unrepresentable where possible; (4) Acceptance -- concrete, code/test-verifiable criteria; (5) Non-goals / out of scope -- what this deliberately does NOT do (bound the fuzzy idea). Keep it bounded and specific. TDD not required for a docs-only plan PR, but the plan must specify the tests the implementation step will add. Do not ask the child to duplicate devloop consensus, CI, PR review, or merge gates; devloop provides those. If no source-grounded objective can be derived at all from an empty/contentless origin, the child must produce no changes with a clear WHY; no-changes is fatal ONLY for that genuinely underivable case, where there is truly nothing to converge."
      }
    },
    {
      "id": "implement-from-plan",
      "title": "Implement the change against the merged plan",
      "content": {
        "kind": "generated",
        "generator": "Read the origin issue and the MERGED converge-to-goal result through the predecessor source_ref (which includes the committed plan document under docs/devloop/plans/) and emit exactly one child issue title and body for the implement-from-plan step. Scope the child PR to implement EXACTLY the converged goal and acceptance criteria in the merged plan, adding the tests the plan specified, all green. Do not re-scope beyond the plan or re-litigate the goal; if the plan is wrong or infeasible against current source, the child must produce no changes with a clear WHY naming the plan defect; no-changes is fatal and blocks the origin. TDD/tests live inside this child PR. Do not ask the child to duplicate devloop consensus, CI, PR review, or merge gates; devloop provides those."
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
