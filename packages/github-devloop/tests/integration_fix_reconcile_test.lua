local h = require("tests.devloop_helpers")
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
local count_calls = h.count_calls

local function origin_marker(version)
  return core.pr_origin_marker("github-devloop/issue/owner/repo/42", "42", "devloop-owner-repo-42-01HY", version, "dev")
end

local function fix_round_version(round)
  local version = reviewing().version
  for _ = 1, round do
    version = core.next_fix_version(version)
  end
  return version
end

local function reject_review_event(version)
  local proposal_id = core.pr_review_proposal_id("owner/repo", 7, version, "feedface")
  return review_reached({
    decision = "reject",
    body = "Review consensus rejects the diff.",
    framing = "Review feedback for " .. tostring(version),
    proposal_id = proposal_id,
    dedup_key = "consensus:" .. proposal_id .. "/review",
  })
end

local function reject_marker(version, framing, created_at)
  local proposal_id = core.pr_review_proposal_id("owner/repo", 7, version, "feedface")
  return {
    body = core.review_result_marker(
      proposal_id,
      "github-devloop/issue/owner/repo/42",
      "reject",
      "consensus:" .. proposal_id .. "/review",
      core.review_reject_framing_digest(framing),
      core.version_fix_round(version)
    ),
    created_at = created_at,
  }
end

return {
  test_review_result_reject_changing_framing_keeps_fixing_past_round_three = function()
    local base_version = reviewing().version
    local round1 = core.next_fix_version(base_version)
    local round2 = core.next_fix_version(round1)
    local round3 = core.next_fix_version(round2)
    local round4 = core.next_fix_version(round3)
    local review_version = round4
    local event = reject_review_event(review_version)
    event.framing = "Fourth review now asks for fixture cleanup."
    local fix_version = core.fix_version_from_review_version(review_version)
    t.eq(core.version_fix_round(review_version) > core.fix_stall_rounds(), true)
    mock_bot_env()
    mock_pr_origin({ origin_marker(base_version) }, "devloop-owner-repo-42-01HY", "feedface")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", review_version),
      reject_marker(round1, "First review asks for payload bounds.", "2026-06-03T01:00:01Z"),
      reject_marker(round2, "Second review asks for source_ref checks.", "2026-06-03T01:00:02Z"),
      reject_marker(round3, "Third review asks for branch rechecks.", "2026-06-03T01:00:03Z"),
    })

    local result = run_review_result(event, opts("fix-progress-changing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    local comment = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request")
    local fixing = find_raise(result.raises, "devloop_fixing")
    t.eq(find_raise(result.raises, "devloop_fix_reconcile"), nil)
    t.eq(label.payload.add_labels[1], "fkst-dev:fixing")
    t.is_true(comment.payload.body:find('state="fixing" version="' .. fix_version .. '"', 1, true) ~= nil)
    t.is_true(comment.payload.body:find('framing_digest="' .. core.review_reject_framing_digest(event.framing) .. '"', 1, true) ~= nil)
    t.eq(fixing.payload.version, fix_version)
  end,

  test_review_result_reject_same_framing_stalls_after_configured_rounds = function()
    local review_version = fix_round_version(core.fix_stall_rounds())
    local event = reject_review_event(review_version)
    local same_framing = "Raising bounds breaks the reliable payload proof."
    event.framing = same_framing
    t.eq(core.version_fix_round(review_version), core.fix_stall_rounds())
    local round1 = fix_round_version(1)
    local round2 = fix_round_version(2)
    mock_bot_env()
    mock_pr_origin({ origin_marker(reviewing().version) }, "devloop-owner-repo-42-01HY", "feedface")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", review_version),
      reject_marker(round1, same_framing, "2026-06-03T01:00:01Z"),
      reject_marker(round2, same_framing, "2026-06-03T01:00:02Z"),
    })

    local result = run_review_result(event, opts("fix-stall-same-framing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
    local reconcile = find_raise(result.raises, "devloop_fix_reconcile").payload
    t.eq(reconcile.schema, "github-devloop.fix-reconcile.v1")
    t.eq(reconcile.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(reconcile.review_proposal_id, event.proposal_id)
    t.eq(reconcile.review_dedup_key, event.dedup_key)
    t.eq(reconcile.issue_version, review_version)
    t.eq(reconcile.head_sha, "feedface")
    t.eq(reconcile.round, core.fix_stall_rounds())
    t.eq(reconcile.pr_number, "7")
    t.eq(reconcile.dedup_key, "fix-reconcile:" .. review_version)
    t.eq(reconcile.source_ref.ref, "owner/repo#pr/7")
    t.eq(core.safe_version_segment(reconcile.issue_version), core.safe_version_segment(review_version))

    mock_bot_env()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(reconcile.proposal_id, "reviewing", review_version),
    })
    local accepted = run_fix_reconcile(reconcile, opts("fix-budget-over-accepted"))
    t.eq(accepted.exit_code, 0)
    t.eq(#accepted.raises, 2)
    local accepted_comment = find_raise(accepted.raises, "github-proxy.github_pr_comment_request").payload
    t.is_true(accepted_comment.body:find(core.state_marker(reconcile.proposal_id, "blocked", reconcile.issue_version), 1, true) ~= nil)
  end,

  test_review_result_reject_max_fix_rounds_blocks_even_when_framing_changes = function()
    local over_version = fix_round_version(core.max_fix_rounds())
    local over_event = reject_review_event(over_version)
    over_event.framing = "Round " .. tostring(core.max_fix_rounds()) .. " has new feedback."
    local previous_a = fix_round_version(core.max_fix_rounds() - 1)
    local previous_b = fix_round_version(core.max_fix_rounds() - 2)
    mock_bot_env()
    mock_pr_origin({ origin_marker(reviewing().version) }, "devloop-owner-repo-42-01HY", "feedface")
    mock_issue_result({ "fkst-dev:reviewing" }, {
      core.state_marker("github-devloop/issue/owner/repo/42", "reviewing", over_version),
      reject_marker(previous_b, "Earlier distinct feedback.", "2026-06-03T01:00:01Z"),
      reject_marker(previous_a, "Previous distinct feedback.", "2026-06-03T01:00:02Z"),
    })

    local over = run_review_result(over_event, opts("fix-max-rounds"))
    t.eq(over.exit_code, 0)
    t.eq(find_raise(over.raises, "devloop_fixing"), nil)
    local reconcile = find_raise(over.raises, "devloop_fix_reconcile").payload
    t.eq(reconcile.issue_version, over_version)
    t.eq(reconcile.round, core.max_fix_rounds())
  end,

  test_fix_reconcile_drop_blocks_reviewing_issue = function()
    local event = fix_reconcile()
    t.eq(core.safe_version_segment(event.issue_version) ~= event.issue_version, true)
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
    t.is_true(comment.body:find("fix-loop-true-stall-after-3-rounds", 1, true) ~= nil)
    t.is_true(comment.body:find(core.state_marker(event.proposal_id, "blocked", event.issue_version), 1, true) ~= nil)
    t.is_true(comment.body:find(core.fix_reconcile_marker(event.proposal_id, event.issue_version, "drop"), 1, true) ~= nil)
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
