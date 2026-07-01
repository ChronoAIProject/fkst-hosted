local devloop_base = require("devloop.base")
local requests_review = require("devloop.requests.review")
local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local t = h.t
local core = h.core
local fixing = h.fixing
local opts = h.opts
local mock_issue_fix_for_event = h.mock_issue_fix_for_event
local mock_pr_fix = h.mock_pr_fix
local mock_existing_fix_worktree = h.mock_existing_fix_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local mock_git_push = h.mock_git_push
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local run_fix = h.run_fix
local gh_argv = require("testkit.gh_argv_mock")
local m_builders = require("devloop.markers.builders")

local function has_commit_subject(subject)
  for _, call in ipairs(t.command_calls()) do
    if gh_argv.argv_contains(call, { "git", "commit", "-m", subject }) then
      return true
    end
  end
  return false
end

local function has_commit_subject_with_prefix(prefix)
  for _, call in ipairs(t.command_calls()) do
    local argv = {}
    if call.program ~= nil and tostring(call.program) ~= "" then
      table.insert(argv, tostring(call.program))
    end
    for _, arg in ipairs(call.args or {}) do
      table.insert(argv, tostring(arg))
    end
    if argv[1] == "git" and argv[2] == "commit" then
      for index, value in ipairs(argv) do
        if value == "-m" and tostring(argv[index + 1] or ""):sub(1, #prefix) == prefix then
          return true
        end
      end
    end
  end
  return false
end

return {
  test_fix_commit_uses_issue_title_subject = function()
    local event = fixing()
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local reject_comment = requests_review.build_review_result_comment_request(core,
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
    local origin_marker = m_builders.pr_origin_marker(core, event.proposal_id, "42", branch, event.version, "dev")
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
    t.is_true(has_commit_subject("auto-fix refs #42: Use issue-derived subjects"))
    t.eq(has_commit_subject_with_prefix("auto-fix #42"), false)
  end,

  test_fix_commit_subject_shell_quotes_single_quote_title = function()
    local event = fixing()
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local reject_comment = requests_review.build_review_result_comment_request(core,
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
    local origin_marker = m_builders.pr_origin_marker(core, event.proposal_id, "42", branch, event.version, "dev")
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
    t.is_true(has_commit_subject("auto-fix refs #42: Don't drop quoted title"))
  end,

  test_fix_commit_subject_falls_back_to_issue_number_when_title_absent = function()
    local event = fixing()
    local branch = devloop_base.implement_branch("owner/repo", "42", event.version)
    local reject_comment = requests_review.build_review_result_comment_request(core,
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
    local origin_marker = m_builders.pr_origin_marker(core, event.proposal_id, "42", branch, event.version, "dev")
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
    t.is_true(has_commit_subject("auto-fix refs #42"))
    t.eq(has_commit_subject_with_prefix("auto-fix #42"), false)
  end,

  test_commit_subject_helpers_keep_message_bounded = function()
    local title = ("long title "):rep(30)
    t.is_true(#payloads_builders.implement_commit_subject(core, "42", { title = title }) <= 200)
    t.is_true(#payloads_builders.fix_commit_subject(core, "42", { title = title }) <= 200)
    t.eq(payloads_builders.implement_commit_subject(core, "42", {}), "auto-implement refs #42")
    t.eq(payloads_builders.fix_commit_subject(core, "42", nil), "auto-fix refs #42")
  end,

  test_commit_subject_helpers_truncate_utf8_safely = function()
    local title = ("界"):rep(80)
    local implement_subject = payloads_builders.implement_commit_subject(core, "42", { title = title })
    local fix_subject = payloads_builders.fix_commit_subject(core, "42", { title = title })
    t.is_true(#implement_subject <= 200)
    t.is_true(#fix_subject <= 200)
    t.is_true(implement_subject:find("界$", 1, false) ~= nil)
    t.is_true(fix_subject:find("界$", 1, false) ~= nil)
  end,
}
