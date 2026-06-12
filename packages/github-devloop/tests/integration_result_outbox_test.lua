local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local reached = h.reached
local run_result = h.run_result
local mock_issue_result = h.mock_issue_result
local find_raise = h.find_raise

return {
  test_consensus_result_ready_marker_heals_missing_declared_effects = function()
    local current = reached()
    mock_issue_result({ "fkst-dev:thinking" }, {
      core.state_marker(current.proposal_id, "ready", current.dedup_key),
    })

    local result = run_result(current, opts("result-outbox-ready-marker-missing-effects"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local ready_raise = find_raise(result.raises, "devloop_ready")
    t.is_true(comment_raise.payload.body:find(core.result_marker(current.proposal_id, current.decision, current.dedup_key), 1, true) ~= nil)
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:ready")
    t.eq(ready_raise.payload.schema, "github-devloop.ready.v1")
    t.eq(ready_raise.payload.dedup_key, core.build_devloop_ready_payload(current).dedup_key)
    t.eq(ready_raise.payload.ready_hand_off.version, ready_raise.payload.dedup_key)
    t.eq(ready_raise.payload.ready_hand_off.effects, "result-marker,ready-label,devloop-ready")
  end,

  test_consensus_result_ready_marker_skips_when_declared_effects_are_complete = function()
    local current = reached()
    mock_issue_result({ "fkst-dev:ready" }, {
      core.state_marker(current.proposal_id, "ready", current.dedup_key),
      core.result_marker(current.proposal_id, current.decision, current.dedup_key),
    })

    local result = run_result(current, opts("result-outbox-complete"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_consensus_result_result_marker_heals_only_missing_label_and_ready_replay = function()
    local current = reached()
    mock_issue_result({ "fkst-dev:thinking" }, {
      core.result_marker(current.proposal_id, current.decision, current.dedup_key),
    })

    local result = run_result(current, opts("result-outbox-result-marker-no-label"))

    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_comment_request"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:ready")
    t.is_true(find_raise(result.raises, "devloop_ready") ~= nil)
  end,
}
