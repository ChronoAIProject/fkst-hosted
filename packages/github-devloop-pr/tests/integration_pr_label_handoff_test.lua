local entity_lib = require("devloop.entity")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local find_raise = h.find_raise
local json_string = h.json_string
local has_value = h.has_value
local mock_default_issue_claim = h.mock_default_issue_claim
local mock_bot_env = h.mock_bot_env

local function run_handoff(handoff, comment_id, name)
  return t.run_department("departments/comment_handoff/main.lua", {
    queue = "github-proxy.github_comment_written",
    payload = {
      schema = "github-proxy.comment-written.v1",
      repo = "owner/repo",
      target = "pr",
      pr_number = 7,
      issue_number = 42,
      comment_id = comment_id or "IC_reviewing_1",
      request_dedup_key = "observe-pr/comment/github-devloop/issue/owner/repo/42/v1/7",
      dedup_key = "observe-pr/comment/github-devloop/issue/owner/repo/42/v1/7/written/" .. tostring(comment_id or "IC_reviewing_1"),
      source_ref = entity_lib.pr_source_ref("owner/repo", 7),
      handoff = handoff,
    },
  }, opts(name))
end

local function reviewing_handoff(version)
  return {
    kind = "github-devloop.reviewing",
    proposal_id = "github-devloop/issue/owner/repo/42",
    pr_number = 7,
    version = version or "v1",
    source_ref = entity_lib.pr_source_ref("owner/repo", 7),
  }
end

local function closed_unmerged_handoff(version)
  return {
    kind = "github-devloop.closed_unmerged",
    proposal_id = "github-devloop/issue/owner/repo/42",
    pr_number = 7,
    version = version or "v1",
    source_ref = entity_lib.pr_source_ref("owner/repo", 7),
  }
end

local function mock_marker_comment(comment_id, body, author_login)
  t.mock_command("gh api --method GET 'repos/owner/repo/issues/comments/" .. tostring(comment_id) .. "'", {
    stdout = '{"body":"' .. json_string(body or "") .. '","user":{"login":"' .. tostring(author_login or "fkst-test-bot") .. '"}}\n',
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_comment_handoff_projects_pr_label_after_reviewing_marker_is_verified = function()
    local handoff = reviewing_handoff("v1")
    mock_bot_env()
    mock_default_issue_claim("owner/repo", 42)
    mock_marker_comment("IC_reviewing_1", core.state_marker(handoff.proposal_id, "reviewing", handoff.version))

    local result = run_handoff(handoff, "IC_reviewing_1", "pr-label-handoff-visible")

    t.eq(result.exit_code, 0)
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.is_true(label ~= nil)
    t.eq(label.payload.target_kind, "pr")
    t.eq(label.payload.target_number, 7)
    t.eq(label.payload.expected_proposal_id, handoff.proposal_id)
    t.eq(label.payload.expected_state, "reviewing")
    t.eq(label.payload.expected_version, handoff.version)
    t.eq(label.payload.require_marker_guard, true)
    t.eq(label.payload.marker_guard.namespace, "github-devloop")
    t.eq(label.payload.marker_guard.marker, "state")
    t.eq(label.payload.marker_guard.version, "v1")
    t.eq(label.payload.marker_guard.match.proposal, handoff.proposal_id)
    t.eq(label.payload.marker_guard.expected.state, "reviewing")
    t.eq(label.payload.marker_guard.expected.version, handoff.version)
    t.eq(label.payload.marker_guard.order_by[1], "marker_order_key")
    t.eq(label.payload.marker_guard.order_by[2], "version_order_key")
    t.eq(label.payload.marker_guard.order_by[3], "stage_rank")
    t.eq(label.payload.add_labels[1], "fkst-dev:reviewing")
    t.is_true(has_value(label.payload.remove_labels, "fkst-dev:pr-open"))
  end,

  test_comment_handoff_projects_pr_label_after_closed_unmerged_marker_is_verified = function()
    local handoff = closed_unmerged_handoff("v1")
    mock_bot_env()
    mock_default_issue_claim("owner/repo", 42)
    mock_marker_comment("IC_closed_unmerged_1", core.state_marker(handoff.proposal_id, "closed-unmerged", handoff.version))

    local result = run_handoff(handoff, "IC_closed_unmerged_1", "pr-label-handoff-closed-unmerged")

    t.eq(result.exit_code, 0)
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.is_true(label ~= nil)
    t.eq(label.payload.target_kind, "pr")
    t.eq(label.payload.expected_state, "closed-unmerged")
    t.eq(label.payload.require_marker_guard, true)
    t.eq(label.payload.marker_guard.expected.state, "closed-unmerged")
    t.eq(label.payload.marker_guard.expected.version, handoff.version)
    t.eq(label.payload.add_labels[1], "fkst-dev:blocked")
    t.is_true(has_value(label.payload.remove_labels, "fkst-dev:reviewing"))
  end,

  test_comment_handoff_retries_pr_label_when_reviewing_marker_is_not_causally_visible = function()
    local handoff = reviewing_handoff("v1")
    mock_bot_env()
    mock_default_issue_claim("owner/repo", 42)
    mock_marker_comment("IC_reviewing_missing", "github-devloop PR ready for review")

    local result = run_handoff(handoff, "IC_reviewing_missing", "pr-label-handoff-missing-marker")

    t.eq(result.exit_code, 1)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
  end,

  test_comment_handoff_skips_pr_label_for_stale_reviewing_marker = function()
    local handoff = reviewing_handoff("v1")
    mock_bot_env()
    mock_default_issue_claim("owner/repo", 42)
    mock_marker_comment("IC_reviewing_stale", core.state_marker(handoff.proposal_id, "merge-ready", handoff.version))

    local result = run_handoff(handoff, "IC_reviewing_stale", "pr-label-handoff-stale-marker")

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
  end,
}
