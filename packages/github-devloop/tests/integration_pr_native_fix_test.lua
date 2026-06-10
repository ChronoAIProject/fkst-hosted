local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local run_fix = h.run_fix
local mock_pr_native_fix = h.mock_pr_native_fix
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls
local find_raise = h.find_raise

local function pr_native_review_reached(extra)
  local version = "pr-native-version"
  local proposal_id = core.pr_review_proposal_id("owner/repo", 7, version, "def456")
  local value = {
    schema = "consensus.consensus_reached.v1",
    proposal_id = proposal_id,
    decision = "reject",
    body = "Review consensus rejects the PR-native diff.",
    blocking_gap = "missing regression guard",
    dedup_key = "consensus:" .. proposal_id .. "/review",
    source_ref = h.pr_source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function pr_native_fixing(extra)
  local event = pr_native_review_reached()
  local proposal_id = core.pr_proposal_id("owner/repo", 7)
  local value = {
    schema = "github-devloop.fixing.v1",
    proposal_id = proposal_id,
    pr_number = 7,
    version = core.fix_version_from_review_version("pr-native-version"),
    review_proposal_id = event.proposal_id,
    review_dedup_key = event.dedup_key,
    reviewed_head_sha = "def456",
    dedup_key = "fixing/" .. proposal_id .. "/v1",
    source_ref = h.pr_source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

return {
  test_pr_native_fix_write_pushes_and_skips_issue_side_effects = function()
    local event = pr_native_fixing()
    local branch = "pr-native-branch"
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      nil,
      event.proposal_id,
      event.version,
      {
        proposal_id = event.review_proposal_id,
        decision = "reject",
        body = "Reject because the PR-native parser must fail closed.",
        blocking_gap = "missing regression guard",
        dedup_key = event.review_dedup_key,
        source_ref = h.pr_source_ref(),
      },
      event.source_ref
    ).body
    mock_bot_env()
    mock_write_env("1")
    mock_pr_native_fix({
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-packages-test/github-devloop/runtime", stderr = "", exit_code = 0 })
    t.mock_command("git worktree list --porcelain", {
      stdout = "worktree /tmp/fix-worktree\nHEAD def456\nbranch refs/heads/" .. branch .. "\n\n",
      stderr = "",
      exit_code = 0,
    })
    mock_implement_codex(0, "fixed PR-native review feedback")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_pr_native_fix({
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, "def456")
    mock_git_push(branch)
    mock_pr_native_fix({}, branch, "feedface")

    local result = run_fix(event, opts("fix-pr-native-write", { FKST_GITHUB_WRITE = "1" }))
    local comment_raise = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    local reviewing_raise = find_raise(result.raises, "devloop_reviewing")
    local expected_version = core.next_fix_version(event.version)
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(count_calls("gh issue view"), 0)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request"), nil)
    t.eq(comment_raise.payload.pr_number, 7)
    t.eq(comment_raise.payload.issue_number, nil)
    t.is_true(comment_raise.payload.body:find(core.fix_marker(event.proposal_id, event.review_proposal_id, event.review_dedup_key, "def456", "feedface"), 1, true) ~= nil)
    t.eq(core.current_state({ comment_raise.payload.body }, event.proposal_id).state, "reviewing")
    t.eq(core.current_state({ comment_raise.payload.body }, event.proposal_id).version, expected_version)
    t.eq(reviewing_raise.payload.proposal_id, event.proposal_id)
    t.eq(reviewing_raise.payload.version, expected_version)
    t.eq(count_calls("git push origin"), 1)
  end,
}
