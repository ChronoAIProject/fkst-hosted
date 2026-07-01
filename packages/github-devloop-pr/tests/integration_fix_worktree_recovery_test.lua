local devloop_base = require("devloop.base")
local requests_review = require("devloop.requests.review")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
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
local mock_missing_fix_worktree = h.mock_missing_fix_worktree
local mock_outside_runtime_fix_worktree = h.mock_outside_runtime_fix_worktree
local mock_write_env = h.mock_write_env
local mock_bot_env = h.mock_bot_env
local count_calls = h.count_calls
local find_raise = h.find_raise
local find_causal_raise = h.find_causal_raise

local function mock_fix_recovery_context(event, branch, origin_marker, reject_comment)
  mock_bot_env()
  mock_write_env("1")
  mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
    core.state_marker(event.proposal_id, "fixing", event.version),
    reject_comment,
  }, branch, event.version)
  mock_pr_fix({ origin_marker }, branch, "def456")
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', { stdout = "/tmp/fkst-packages-test/github-devloop/runtime", stderr = "", exit_code = 0 })
end

local function mock_fix_writeback(event, branch, origin_marker)
  mock_implement_codex(0, "fixed after rebuilding worktree")
  mock_git_status(" M packages/github-devloop/core.lua\n")
  mock_git_commit("feedface", branch)
  mock_write_env("1")
  mock_issue_fix_for_event(event, { "fkst-dev:fixing" }, {
    core.state_marker(event.proposal_id, "fixing", event.version),
    requests_review.build_review_result_comment_request(core,
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
    ).body,
  }, branch, event.version)
  mock_pr_fix({ origin_marker }, branch, "def456")
  mock_git_push(branch)
  mock_pr_fix({ origin_marker }, branch, "feedface")
end

return {
  test_fix_rebuilds_missing_recorded_worktree_under_current_runtime_root = function()
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
    mock_fix_recovery_context(event, branch, origin_marker, reject_comment)
    mock_missing_fix_worktree(branch, "def456")
    mock_fix_writeback(event, branch, origin_marker)

    local result = run_fix(event, opts("fix-rebuild-missing-worktree", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_label_request").payload.add_labels[1], "fkst-dev:reviewing")
    t.eq(find_causal_raise(result, "devloop_reviewing").payload.version, core.next_fix_version(event.version))
    t.eq(count_calls("git worktree prune"), 1)
    t.eq(count_calls("git fetch 'origin' '" .. branch .. "'"), 1)
    t.eq(count_calls("git worktree add --force -B"), 1)
    t.eq(count_calls("refs/remotes/'origin'/'" .. branch .. "'"), 1)

    local found_current_root_worktree = false
    for _, call in ipairs(t.command_calls()) do
      if call.rendered:find("codex exec", 1, true) ~= nil
        and call.rendered:find("/tmp/fkst-packages-test/github-devloop/runtime/worktrees/devloop-owner-repo-42-", 1, true) ~= nil then
        found_current_root_worktree = true
      end
    end
    t.eq(found_current_root_worktree, true)
  end,

  test_fix_removes_existing_outside_runtime_worktree_before_rebuild = function()
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
    mock_fix_recovery_context(event, branch, origin_marker, reject_comment)
    mock_outside_runtime_fix_worktree(branch, "def456")
    mock_fix_writeback(event, branch, origin_marker)

    local result = run_fix(event, opts("fix-rebuild-outside-runtime-worktree", { FKST_GITHUB_WRITE = "1" }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    t.eq(count_calls("git worktree remove --force"), 1)
    t.eq(count_calls("git worktree prune"), 0)
    t.eq(count_calls("git fetch 'origin' '" .. branch .. "'"), 1)
    t.eq(count_calls("git worktree add --force -B"), 1)

    local found_current_root_worktree = false
    for _, call in ipairs(t.command_calls()) do
      if call.rendered:find("codex exec", 1, true) ~= nil
        and call.rendered:find("/tmp/fkst-packages-test/github-devloop/runtime/worktrees/devloop-owner-repo-42-", 1, true) ~= nil then
        found_current_root_worktree = true
      end
    end
    t.eq(found_current_root_worktree, true)
  end,
}
