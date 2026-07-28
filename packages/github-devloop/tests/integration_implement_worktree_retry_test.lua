local h = require("tests.devloop_helpers")
local gh_argv = require("testkit.gh_argv_mock")

local t = h.t
local core = h.core
local ready = h.ready
local opts = h.opts
local run_implement = h.run_implement
local mock_issue_implement = h.mock_issue_implement
local mock_fresh_implement_worktree = h.mock_fresh_implement_worktree
local mock_existing_dirty_implement_worktree_reuse = h.mock_existing_dirty_implement_worktree_reuse
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local deterministic_branch_for = h.deterministic_branch_for
local count_calls = h.count_calls
local find_raise = h.find_raise

local function find_comment_with(raises, text)
  return find_raise(raises, "github-proxy.github_issue_comment_request", function(payload)
    return tostring(payload.body or ""):find(text, 1, true) ~= nil
  end)
end

local function mock_pr_child_created(branch)
  local list = core.gh_pr_list_head_base_cmd("owner/repo", branch, "dev")
  t.mock_command(list, {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh pr create", {
    stdout = "https://github.example/owner/repo/pull/7\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(list, {
    stdout = '[{"number":7,"head":{"ref":"' .. branch
      .. '","sha":"def456"},"base":{"ref":"dev"},"state":"open"}]\n',
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_implement_status_timeout_redelivery_preserves_output_and_publishes = function()
    local event = ready()
    local branch = deterministic_branch_for(event)
    local run_opts = opts("implement-status-timeout-redelivery", {
      FKST_GITHUB_WRITE = "1",
    })

    mock_issue_implement({ "fkst-dev:ready" })
    mock_fresh_implement_worktree()
    mock_implement_codex(0, "Implemented the requested change.")
    mock_git_status("", 124, "status probe timed out")

    local first = run_implement(event, run_opts)
    t.is_true(first.exit_code ~= 0)
    t.is_true(tostring(first.error or first.stderr or ""):find("git-status-failed", 1, true) ~= nil)

    mock_issue_implement({ "fkst-dev:ready" })
    mock_existing_dirty_implement_worktree_reuse(nil, branch, "0")
    mock_implement_codex(0, "Implemented the requested change.")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("def456", branch)
    mock_git_push(branch)
    mock_pr_child_created(branch)
    for _ = 1, 6 do
      mock_write_env("1")
    end
    mock_bot_env()

    local retry = run_implement(event, run_opts)
    t.eq(retry.exit_code, 0)
    t.eq(count_calls("reset --hard"), 1)
    t.eq(count_calls("clean -fd"), 1)
    t.eq(count_calls("merge --no-edit 'abc123'"), 1)
    t.eq(count_calls("status --porcelain"), 3)
    t.eq(count_calls("codex exec"), 2)
    t.eq(find_comment_with(retry.raises, "implementation failed: no-changes"), nil)
    t.is_true(find_raise(retry.raises, "github-proxy.github_pr_comment_request", function(payload)
      return tostring(payload.body or ""):find('state="pr-open"', 1, true) ~= nil
    end) ~= nil)
    t.is_true(find_comment_with(retry.raises, "fkst:github-devloop:pr-delegation:v1") ~= nil)

    local codex_calls = {}
    local push_calls = 0
    for _, call in ipairs(t.command_calls()) do
      if tostring(call.rendered or ""):find("codex exec", 1, true) ~= nil then
        table.insert(codex_calls, call)
      end
      if gh_argv.argv_contains(call, { "git", "push", "origin" }) then
        push_calls = push_calls + 1
      end
    end
    t.eq(push_calls, 1)
    t.eq(#codex_calls, 2)
    t.eq(codex_calls[1].rendered, codex_calls[2].rendered)
  end,
}
