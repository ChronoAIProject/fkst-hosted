local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local unresolved = h.unresolved
local run_loop = h.run_loop
local mock_issue_loop = h.mock_issue_loop
local find_raise = h.find_raise

return {
  test_loop_round_cap_uses_proposal_budget_when_version_and_source_ref_drift = function()
    local cap = core.max_converge_rounds()
    local base_version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local drift_version = base_version .. "/drifted"
    local event = unresolved({
      dedup_key = base_version .. "/loop/6",
      round = 6,
      narrowed_question = "Question 6 current lineage",
      angle_digests = {
        { angle = "minimal", verdict = "abstain", digest = "digest-6" },
      },
    })
    local current_digest = core.source_ref_digest(event.source_ref)
    local drift_digest = core.source_ref_digest({ kind = "external", ref = "owner/repo#issue/42?drift=1" })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.converge_round_marker(event.proposal_id, drift_version, drift_digest, cap, drift_version .. "/loop/" .. tostring(cap), "Question " .. tostring(cap) .. " drifted", {
        { angle = "minimal", verdict = "abstain", digest = "digest-" .. tostring(cap) },
      }),
    })

    local result = run_loop(event, opts("loop-budget-drift-cap"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(result.raises[1].payload.body:find('round="6"', 1, true) ~= nil)
    t.eq(result.raises[2].queue, "devloop_reconcile")
    local reconcile_raise = find_raise(result.raises, "devloop_reconcile").payload
    t.eq(reconcile_raise.round, 6)
    t.eq(reconcile_raise.dedup_key, "reconcile:" .. base_version .. "/loop/6")
  end,
}
