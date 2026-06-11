local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local fixing = h.fixing
local run_fix = h.run_fix
local mock_issue_fix_for_event = h.mock_issue_fix_for_event
local mock_pr_fix = h.mock_pr_fix
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local mock_existing_fix_worktree = h.mock_existing_fix_worktree
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls
local find_raise = h.find_raise

return {
  test_fix_merges_current_integration_before_codex = function()
    local event = fixing({ gate_baseline_sha = "abc123", gate_failure_excerpt = "rollup-red: test: COMPLETED/FAILURE" })
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = "github-devloop merge gate failed: rollup-red: test: COMPLETED/FAILURE"
      .. "\n" .. core.state_marker(event.proposal_id, "fixing", event.version)
      .. "\n" .. core.merge_gate_marker(
        event.proposal_id,
        event.pr_number,
        event.version,
        event.review_proposal_id,
        event.review_dedup_key,
        event.reviewed_head_sha,
        event.gate_baseline_sha,
        "rollup-red"
      )
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    local worktree = mock_existing_fix_worktree(branch, "def456", nil, {
      sha = "abc123",
      exit_code = 1,
      stdout = "",
      stderr = "CONFLICT (content): Merge conflict in packages/github-devloop/core.lua\n",
      unmerged_stdout = "100644 abc123 1\tpackages/github-devloop/core.lua\n",
    })
    mock_implement_codex(0, "resolved merge product failure")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("feedface", branch)
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version)
    mock_pr_fix({ origin_marker }, branch, "def456")
    mock_git_push(branch)
    mock_pr_fix({ origin_marker }, branch, "feedface")

    local result = run_fix(event, opts("fix-merge-product-before-codex", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 3)
    t.eq(find_raise(result.raises, "devloop_reviewing").payload.version, core.next_fix_version(event.version))

    local merge_index = nil
    local codex_index = nil
    for index, call in ipairs(t.command_calls()) do
      if call.rendered:find("git -C '" .. worktree .. "' merge --no-edit 'abc123'", 1, true) ~= nil then
        merge_index = index
      elseif call.rendered:find("codex exec", 1, true) ~= nil then
        codex_index = index
      end
    end
    t.is_true(merge_index ~= nil)
    t.is_true(codex_index ~= nil)
    t.is_true(merge_index < codex_index)
    t.eq(count_calls("git fetch 'origin' 'dev'"), 1)
    t.eq(count_calls("refs/remotes/'origin'/'dev'^{commit}"), 0)
    t.eq(count_calls("merge --no-edit 'abc123'"), 1)
  end,
}
