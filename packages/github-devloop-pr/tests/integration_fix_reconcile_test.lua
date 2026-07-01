local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local transition_version = require("contract.transition_version")
local conv_reconcile = require("devloop.convergence.reconcile")
local t = h.t
local core = h.core
local opts = h.opts
local reviewing = h.reviewing
local review_reached = h.review_reached
local fix_reconcile = h.fix_reconcile
local run_review_result = h.run_review_result
local run_fix_reconcile = h.run_fix_reconcile
local mock_bot_env = h.mock_bot_env
local mock_pr_origin = h.mock_pr_origin
local mock_issue_result = h.mock_issue_result
local mock_issue_review = h.mock_issue_review
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise
local count_calls = h.count_calls
local config = require("devloop.config")
local m_builders = require("devloop.markers.builders")

local function origin_marker(version)
  return m_builders.pr_origin_marker(core, "github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", version, "dev")
end

local function fix_round_version(round)
  local version = reviewing().version
  for _ = 1, round do
    version = core.next_fix_version(version)
  end
  return version
end

local function reject_review_event(version)
  local proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, version, "feedface")
  return review_reached({
    decision = "reject",
    body = "Review consensus rejects the diff.",
    blocking_gap = "missing regression guard",
    framing = "Review feedback for " .. tostring(version),
    proposal_id = proposal_id,
    dedup_key = "consensus:" .. proposal_id .. "/review",
  })
end

local function reject_marker(version, created_at)
  local proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, version, "feedface")
  return {
    body = m_builders.review_result_marker(core, 
      proposal_id,
      "github-devloop/issue/owner/repo/42",
      "reject",
      "consensus:" .. proposal_id .. "/review",
      core.version_fix_round(version),
      "missing regression guard"
    ),
    created_at = created_at,
  }
end

return {
  test_review_result_reject_same_framing_below_max_keeps_fixing = function()
    local review_version = fix_round_version(config.max_fix_rounds(core) - 1)
    local event = reject_review_event(review_version)
    event.framing = "Raising bounds breaks the reliable payload proof."
    local fix_version = core.fix_version_from_review_version(review_version)
    t.eq(core.version_fix_round(review_version), config.max_fix_rounds(core) - 1)
    mock_bot_env()
    mock_pr_origin({ origin_marker(reviewing().version) }, "devloop-owner-repo-42-01HY", "feedface")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", review_version),
      reject_marker(fix_round_version(1), "2026-06-03T01:00:01Z"),
      reject_marker(fix_round_version(2), "2026-06-03T01:00:02Z"),
      reject_marker(fix_round_version(3), "2026-06-03T01:00:03Z"),
    })

    local result = run_review_result(event, opts("fix-progress-same-framing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local fixing = find_causal_raise(result, "devloop_fixing")
    t.eq(find_raise(result.raises, "devloop_fix_reconcile"), nil)
    t.eq(label.payload.add_labels[1], "fkst-dev:fixing")
    t.is_true(comment.payload.body:find('state="fixing" version="' .. fix_version .. '"', 1, true) ~= nil)
    t.is_true(comment.payload.body:find('fix_round="' .. tostring(core.version_fix_round(fix_version)) .. '"', 1, true) ~= nil)
    t.eq(fixing.payload.version, fix_version)
    t.eq(fixing.payload.framing, event.framing)
  end,

  test_review_result_reject_max_fix_rounds_blocks_even_when_framing_changes = function()
    local over_version = fix_round_version(config.max_fix_rounds(core))
    local over_event = reject_review_event(over_version)
    over_event.framing = "Round " .. tostring(config.max_fix_rounds(core)) .. " has new feedback."
    local previous_a = fix_round_version(config.max_fix_rounds(core) - 1)
    local previous_b = fix_round_version(config.max_fix_rounds(core) - 2)
    mock_bot_env()
    mock_pr_origin({ origin_marker(reviewing().version) }, "devloop-owner-repo-42-01HY", "feedface")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", over_version),
      reject_marker(previous_b, "2026-06-03T01:00:01Z"),
      reject_marker(previous_a, "2026-06-03T01:00:02Z"),
    })

    local over = run_review_result(over_event, opts("fix-max-rounds"))
    t.eq(over.exit_code, 0)
    t.eq(find_raise(over.raises, "devloop_fixing"), nil)
    local reconcile = find_raise(over.raises, "devloop_fix_reconcile").payload
    local decompose = find_raise(over.raises, "github-devloop-decompose.devloop_decompose").payload
    t.eq(reconcile.issue_version, over_version)
    t.eq(reconcile.round, config.max_fix_rounds(core))
    t.eq(decompose.schema, "github-devloop.decompose.v1")
    t.eq(decompose.proposal_id, reconcile.proposal_id)
    t.eq(decompose.version, reconcile.issue_version)
    t.eq(decompose.pr_number, reconcile.pr_number)
  end,

  test_fix_reconcile_drop_blocks_reviewing_issue = function()
    local event = fix_reconcile()
    t.eq(transition_version.safe_version_segment(event.issue_version) ~= event.issue_version, true)
    mock_bot_env()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", event.issue_version),
    })

    local result = run_fix_reconcile(event, opts("fix-reconcile-drop"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment = find_raise(result.raises, "github-proxy.github_pr_comment_request").payload
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request").payload
    t.is_true(comment.body:find("github-devloop fix reconcile action: drop", 1, true) ~= nil)
    t.is_true(comment.body:find("fix-loop-max-rounds-after-3-rounds", 1, true) ~= nil)
    t.is_true(comment.body:find(core.state_marker(event.proposal_id, "blocked", event.issue_version), 1, true) ~= nil)
    t.is_true(comment.body:find(conv_reconcile.fix_reconcile_marker(core, event.proposal_id, event.issue_version, "drop"), 1, true) ~= nil)
    t.eq(label.add_labels[1], "fkst-dev:blocked")
    t.eq(label.remove_labels[1], "fkst-dev:thinking")
    t.eq(count_calls("codex exec"), 0)
  end,

  test_fix_reconcile_visible_marker_is_idempotent = function()
    local event = fix_reconcile()
    mock_bot_env()
    mock_issue_review({ "fkst-dev:blocked" }, {
      core.build_fix_reconcile_comment_request("owner/repo", "42", event, "drop", "already done").body,
    })

    local result = run_fix_reconcile(event, opts("fix-reconcile-idempotent"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("codex exec"), 0)
  end,

  test_fix_reconcile_requires_visible_reviewing_marker = function()
    local event = fix_reconcile()
    mock_bot_env()
    mock_issue_review({ "fkst-dev:enabled" }, {})

    local result = run_fix_reconcile(event, opts("fix-reconcile-pending-reviewing"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_fix_reconcile_skips_when_already_terminal = function()
    local event = fix_reconcile()
    mock_bot_env()
    mock_issue_review({ "fkst-dev:blocked" }, {
      core.state_marker(event.proposal_id, "blocked", event.issue_version),
    })

    local result = run_fix_reconcile(event, opts("fix-reconcile-terminal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("gh issue comment"), 0)
    t.eq(count_calls("gh issue edit"), 0)
  end,
}
