local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local issue = h.issue
local reached = h.reached
local run_observe = h.run_observe
local run_implement = h.run_implement
local mock_issue_state = h.mock_issue_state
local mock_issue_implement_raw = h.mock_issue_implement_raw
local mock_existing_empty_implement_worktree = h.mock_existing_empty_implement_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local find_raise = h.find_raise

local function trusted_command(id)
  return {
    id = id or "IC_reimplement_1",
    body = "fkst: reimplement",
    author_login = "fkst-test-bot",
    created_at = "2026-06-04T03:00:00Z",
  }
end

local function forged_command()
  local command = trusted_command("IC_reimplement_forged")
  command.author_login = "mallory"
  return command
end

local function impl_failed_comments(event, reason, attempt, command)
  local version = core.build_devloop_ready_payload(event).dedup_key
  local comments = {
    core.state_marker(event.proposal_id, "impl-failed", version),
    core.impl_failure_marker(event.proposal_id, version, reason or "codex-failed", attempt),
  }
  if command ~= nil then
    table.insert(comments, command)
  end
  return comments
end

return {
  test_observe_autoretries_codex_failed_once = function()
    local event = reached()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:impl-failed" }, "OPEN", impl_failed_comments(event, "codex-failed", 1))

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:impl-failed" } }), opts("observe-impl-failed-retry"))
    t.eq(result.exit_code, 0)
    local ready = find_raise(result.raises, "devloop_ready")
    t.is_true(ready ~= nil)
    t.eq(ready.payload.dedup_key, core.build_devloop_ready_payload(event).dedup_key)
    t.eq(ready.payload.impl_retry_attempt, 2)
  end,

  test_observe_autoretries_non_descendant_head_once = function()
    local event = reached()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:impl-failed" }, "OPEN", impl_failed_comments(event, "non-descendant-head", 1))

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:impl-failed" } }), opts("observe-non-descendant-head-retry"))
    t.eq(result.exit_code, 0)
    local ready = find_raise(result.raises, "devloop_ready")
    t.is_true(ready ~= nil)
    t.eq(ready.payload.dedup_key, core.build_devloop_ready_payload(event).dedup_key)
    t.eq(ready.payload.impl_retry_attempt, 2)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_comment_request"), nil)
  end,

  test_observe_stops_after_bounded_codex_failed_retry = function()
    local event = reached()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:impl-failed" }, "OPEN", impl_failed_comments(event, "codex-failed", 2))

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:impl-failed" } }), opts("observe-impl-failed-limit"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
  end,

  test_observe_stops_after_bounded_non_descendant_head_retry = function()
    local event = reached()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:impl-failed" }, "OPEN", impl_failed_comments(event, "non-descendant-head", 2))

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:impl-failed" } }), opts("observe-non-descendant-head-limit"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
  end,

  test_reimplement_command_reenters_after_retry_limit = function()
    local event = reached()
    local command = trusted_command()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:impl-failed" }, "OPEN", impl_failed_comments(event, "codex-failed", 2, command))

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:impl-failed" } }), opts("operator-reimplement"))
    t.eq(result.exit_code, 0)
    local ready = find_raise(result.raises, "devloop_ready")
    t.is_true(ready ~= nil)
    t.eq(ready.payload.dedup_key, core.build_devloop_ready_payload(event).dedup_key)
    t.eq(ready.payload.impl_retry_attempt, 3)
    local response = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(response.payload.body:find("operator command accepted: reimplement", 1, true) ~= nil)
    t.is_true(response.payload.body:find('command="reimplement"', 1, true) ~= nil)
  end,

  test_forged_reimplement_command_is_ignored = function()
    local event = reached()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:impl-failed" }, "OPEN", impl_failed_comments(event, "codex-failed", 2, forged_command()))

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:impl-failed" } }), opts("operator-reimplement-forged"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
  end,

  test_retry_implementation_writes_attempt_version = function()
    local event = reached()
    local ready = core.build_devloop_ready_payload(event)
    ready.impl_retry_attempt = 2
    mock_issue_implement_raw({ "fkst-dev:impl-failed" }, {
      core.state_marker(event.proposal_id, "impl-failed", ready.dedup_key),
      core.impl_failure_marker(event.proposal_id, ready.dedup_key, "codex-failed", 1),
    })
    mock_existing_empty_implement_worktree()
    mock_implement_codex(0, "implemented")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit(nil, core.implement_branch("owner/repo", "42", ready.dedup_key))
    mock_issue_implement_raw({ "fkst-dev:impl-failed" }, {
      core.state_marker(event.proposal_id, "impl-failed", ready.dedup_key),
      core.impl_failure_marker(event.proposal_id, ready.dedup_key, "codex-failed", 1),
    })

    local result = run_implement(ready, opts("implement-retry-success"))
    t.eq(result.exit_code, 0)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request", function(payload)
      return tostring(payload.body or ""):find("github-devloop implementation started", 1, true) ~= nil
    end)
    t.is_true(comment ~= nil)
    t.is_true(comment.payload.body:find(core.state_marker(event.proposal_id, "implementing", ready.dedup_key .. "/reimplement/2"), 1, true) ~= nil)
  end,
}
