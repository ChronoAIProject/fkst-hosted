local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local merge_ready = h.merge_ready
local run_merge = h.run_merge
local mock_issue_merge = h.mock_issue_merge
local merge_comments = h.merge_comments
local mock_pr_merge = h.mock_pr_merge
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls
local find_raise = h.find_raise
local config = require("devloop.config")
local m_builders = require("devloop.markers.builders")

local function mock_base_head_for_stale_mergeability()
  t.mock_command("git fetch origin dev", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("git rev-parse --verify 'refs/remotes/origin/dev^{commit}'", {
    stdout = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git merge-base --is-ancestor aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa def456", {
    stdout = "",
    stderr = "",
    exit_code = 1,
  })
end

local function max_fix_round_merge_ready()
  local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
  for n = 1, config.max_fix_rounds(core) do
    version = version .. "/fix/" .. tostring(n)
  end
  local review_proposal_id = devloop_base.pr_review_proposal_id("owner/repo", 7, version, "def456")
  return merge_ready({
    version = version,
    review_proposal_id = review_proposal_id,
    review_dedup_key = "consensus:" .. review_proposal_id .. "/review",
  })
end

return {
  test_merge_conflict_churn_at_max_fix_rounds_decomposes_without_another_fix = function()
    local event = max_fix_round_merge_ready()
    local origin_marker = m_builders.pr_origin_marker(core, event.proposal_id, "42", "devloop-owner-repo-42-01HY", event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_write_env("1")
    mock_issue_merge({ "fkst-dev:merge-ready" }, merge_comments(event))
    mock_pr_merge({ origin_marker }, "devloop-owner-repo-42-01HY", "def456", "OPEN", "owner/repo", false, "MERGEABLE", "DIRTY")
    mock_base_head_for_stale_mergeability()

    local result = run_merge(event, opts("merge-conflict-max-fix-rounds", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(count_calls("gh pr merge"), 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_pr_comment_request"), nil)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    local reconcile = find_raise(result.raises, "devloop_fix_reconcile")
    local decompose = find_raise(result.raises, "github-devloop-decompose.devloop_decompose")
    t.eq(reconcile.payload.issue_version, event.version)
    t.eq(reconcile.payload.round, config.max_fix_rounds(core))
    t.eq(reconcile.payload.pr_number, event.pr_number)
    t.eq(decompose.payload.version, event.version)
    t.eq(decompose.payload.round, config.max_fix_rounds(core))
    t.eq(decompose.payload.pr_number, event.pr_number)
    t.eq(decompose.payload.review_proposal_id, event.review_proposal_id)
    t.eq(decompose.payload.review_dedup_key, event.review_dedup_key)
    t.eq(decompose.payload.head_sha, event.reviewed_head_sha)
  end,
}
