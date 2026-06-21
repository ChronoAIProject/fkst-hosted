local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local ready = h.ready
local opts = h.opts
local mock_issue_implement = h.mock_issue_implement
local mock_fresh_implement_worktree = h.mock_fresh_implement_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local run_implement = h.run_implement
local gh_argv = require("tests.gh_argv_mock_helpers")

local function has_commit_subject(subject)
  for _, call in ipairs(t.command_calls()) do
    if gh_argv.argv_contains(call, { "git", "commit", "-m", subject }) then
      return true
    end
  end
  return false
end

return {
  test_implement_commit_uses_issue_title_subject = function()
    local event = ready()
    local branch = core.implement_branch("owner/repo", "42", event.dedup_key)
    mock_issue_implement({ "fkst-dev:ready" }, {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
    }, {
      title = "Use issue-derived subjects",
    })
    mock_fresh_implement_worktree()
    mock_implement_codex()
    mock_git_status(" M packages/github-devloop/departments/implement/main.lua\n")
    mock_git_commit("def456", branch)

    local result = run_implement(event, opts("implement-commit-subject"))
    t.eq(result.exit_code, 0)
    t.is_true(has_commit_subject("auto-implement #42: Use issue-derived subjects"))
  end,

  test_implement_commit_subject_shell_quotes_single_quote_title = function()
    local event = ready()
    local branch = core.implement_branch("owner/repo", "42", event.dedup_key)
    mock_issue_implement({ "fkst-dev:ready" }, {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
    }, {
      title = "Don't drop quoted title",
    })
    mock_fresh_implement_worktree()
    mock_implement_codex()
    mock_git_status(" M packages/github-devloop/departments/implement/main.lua\n")
    mock_git_commit("def456", branch)

    local result = run_implement(event, opts("implement-commit-subject-quote"))
    t.eq(result.exit_code, 0)
    t.is_true(has_commit_subject("auto-implement #42: Don't drop quoted title"))
  end,

  test_implement_commit_subject_preserves_chinese_title = function()
    local event = ready()
    local branch = core.implement_branch("owner/repo", "42", event.dedup_key)
    mock_issue_implement({ "fkst-dev:ready" }, {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
    }, {
      title = "派生提交标题",
    })
    mock_fresh_implement_worktree()
    mock_implement_codex()
    mock_git_status(" M packages/github-devloop/departments/implement/main.lua\n")
    mock_git_commit("def456", branch)

    local result = run_implement(event, opts("implement-commit-subject-chinese"))
    t.eq(result.exit_code, 0)
    t.is_true(has_commit_subject("auto-implement #42: 派生提交标题"))
  end,

  test_implement_commit_subject_falls_back_to_issue_number = function()
    local event = ready()
    local branch = core.implement_branch("owner/repo", "42", event.dedup_key)
    mock_issue_implement({ "fkst-dev:ready" }, {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
    }, {
      title = "",
    })
    mock_fresh_implement_worktree()
    mock_implement_codex()
    mock_git_status(" M packages/github-devloop/departments/implement/main.lua\n")
    mock_git_commit("def456", branch)

    local result = run_implement(event, opts("implement-commit-subject-fallback"))
    t.eq(result.exit_code, 0)
    t.is_true(has_commit_subject("auto-implement #42"))
  end,

  test_implement_commit_subject_falls_back_when_title_fetch_fails = function()
    local event = ready()
    local branch = core.implement_branch("owner/repo", "42", event.dedup_key)
    mock_issue_implement({ "fkst-dev:ready" }, {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
    }, {
      title = "CAS title remains readable",
      commit_title_error = "subject title fetch failed",
    })
    mock_fresh_implement_worktree()
    mock_implement_codex()
    mock_git_status(" M packages/github-devloop/departments/implement/main.lua\n")
    mock_git_commit("def456", branch)

    local result = run_implement(event, opts("implement-commit-subject-fetch-fallback"))
    t.eq(result.exit_code, 0)
    t.is_true(has_commit_subject("auto-implement #42"))
  end,
}
