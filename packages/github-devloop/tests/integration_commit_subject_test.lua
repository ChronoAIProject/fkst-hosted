local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local ready = h.ready
local fixing = h.fixing
local opts = h.opts
local mock_issue_implement = h.mock_issue_implement
local mock_issue_fix_for_event = h.mock_issue_fix_for_event
local mock_pr_fix = h.mock_pr_fix
local mock_fresh_implement_worktree = h.mock_fresh_implement_worktree
local mock_existing_fix_worktree = h.mock_existing_fix_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local run_implement = h.run_implement
local run_fix = h.run_fix

local function has_commit_subject(subject)
  local rendered_subject = "commit -m '" .. tostring(subject):gsub("'", "'\\''") .. "'"
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find(rendered_subject, 1, true) ~= nil then
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

  test_fix_commit_uses_issue_title_subject = function()
    local event = fixing()
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      {
        proposal_id = event.review_proposal_id,
        decision = "reject",
        body = "Reject because parser must fail closed.",
        blocking_gap = "missing regression guard",
        dedup_key = event.review_dedup_key,
        source_ref = { kind = "external", ref = "owner/repo#pr/7" },
      },
      event.source_ref
    ).body
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version, {
      title = "Use issue-derived subjects",
    })
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, "def456")
    mock_implement_codex(0, "fixed review feedback")
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

    local result = run_fix(event, opts("fix-commit-subject", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.is_true(has_commit_subject("auto-fix #42: Use issue-derived subjects"))
  end,

  test_fix_commit_subject_shell_quotes_single_quote_title = function()
    local event = fixing()
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      {
        proposal_id = event.review_proposal_id,
        decision = "reject",
        body = "Reject because parser must fail closed.",
        blocking_gap = "missing regression guard",
        dedup_key = event.review_dedup_key,
        source_ref = { kind = "external", ref = "owner/repo#pr/7" },
      },
      event.source_ref
    ).body
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version, {
      title = "Don't drop quoted title",
    })
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, "def456")
    mock_implement_codex(0, "fixed review feedback")
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

    local result = run_fix(event, opts("fix-commit-subject-quote", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.is_true(has_commit_subject("auto-fix #42: Don't drop quoted title"))
  end,

  test_fix_commit_subject_falls_back_to_issue_number_when_title_absent = function()
    local event = fixing()
    local branch = core.implement_branch("owner/repo", "42", event.version)
    local reject_comment = core.build_review_result_comment_request(
      "owner/repo",
      "42",
      event.proposal_id,
      event.version,
      {
        proposal_id = event.review_proposal_id,
        decision = "reject",
        body = "Reject because parser must fail closed.",
        blocking_gap = "missing regression guard",
        dedup_key = event.review_dedup_key,
        source_ref = { kind = "external", ref = "owner/repo#pr/7" },
      },
      event.source_ref
    ).body
    local origin_marker = core.pr_origin_marker(event.proposal_id, "42", branch, event.version, "dev")
    mock_bot_env()
    mock_write_env("1")
    mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
      core.state_marker(event.proposal_id, "fixing", event.version),
      reject_comment,
    }, branch, event.version, {
      title = "",
    })
    mock_pr_fix({ origin_marker }, branch, "def456")
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
    mock_existing_fix_worktree(branch, "def456")
    mock_implement_codex(0, "fixed review feedback")
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

    local result = run_fix(event, opts("fix-commit-subject-fallback", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.is_true(has_commit_subject("auto-fix #42"))
  end,

  test_commit_subject_helpers_keep_message_bounded = function()
    local title = ("long title "):rep(30)
    t.is_true(#core.implement_commit_subject("42", { title = title }) <= 200)
    t.is_true(#core.fix_commit_subject("42", { title = title }) <= 200)
    t.eq(core.implement_commit_subject("42", {}), "auto-implement #42")
    t.eq(core.fix_commit_subject("42", nil), "auto-fix #42")
  end,

  test_commit_subject_helpers_truncate_utf8_safely = function()
    local title = ("界"):rep(80)
    local implement_subject = core.implement_commit_subject("42", { title = title })
    local fix_subject = core.fix_commit_subject("42", { title = title })
    t.is_true(#implement_subject <= 200)
    t.is_true(#fix_subject <= 200)
    t.is_true(implement_subject:find("界$", 1, false) ~= nil)
    t.is_true(fix_subject:find("界$", 1, false) ~= nil)
  end,
}
