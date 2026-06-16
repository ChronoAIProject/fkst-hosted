local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local reconcile = h.reconcile
local review_reconcile = h.review_reconcile
local run_reconcile = h.run_reconcile
local run_review_reconcile = h.run_review_reconcile
local mock_issue_reconcile = h.mock_issue_reconcile
local mock_issue_review = h.mock_issue_review
local mock_bot_env = h.mock_bot_env
local find_raise = h.find_raise
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local function run_timeout_reconcile(payload, run_opts)
  return t.run_department("departments/reconcile/main.lua", {
    queue = "devloop_timeout_reconcile",
    payload = payload,
  }, run_opts)
end

return {
  test_thinking_reconcile_blocks_when_live_version_outranks_convergence_base = function()
    local event = reconcile()
    local state_version = "github-devloop/issue/owner/repo/42/2026-06-14T05-22-55Z/intake/1287859418"
    mock_issue_reconcile({ "fkst-dev:thinking" }, {
      core.state_marker(event.proposal_id, "thinking", state_version),
    })

    local result = run_reconcile(event, opts("reconcile-terminal-thinking"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request").payload
    local version = core.reconcile_terminal_state_version(state_version, event.round)
    t.eq(core.versioned_transition_status({ state = "thinking", version = state_version }, { "thinking" }, "blocked", version), "apply")
    t.is_true(comment.body:find(core.state_marker(event.proposal_id, "blocked", version), 1, true) ~= nil)

    mock_issue_reconcile({ "fkst-dev:blocked" }, { comment.body })
    local idempotent = run_reconcile(event, opts("reconcile-terminal-thinking-idempotent"))
    t.eq(idempotent.exit_code, 0)
    t.eq(#idempotent.raises, 0)
  end,

  test_thinking_reconcile_does_not_override_advanced_state = function()
    local event = reconcile()
    local state_version = core.reconcile_terminal_state_version("github-devloop/issue/owner/repo/42/2026-06-14T05-22-55Z/intake/1287859418", event.round)
    mock_issue_reconcile({ "fkst-dev:ready" }, {
      core.state_marker(event.proposal_id, "ready", state_version),
    })

    local ready_result = run_reconcile(event, opts("reconcile-terminal-ready"))
    t.eq(ready_result.exit_code, 0)
    t.eq(#ready_result.raises, 0)

    mock_issue_reconcile({ "fkst-dev:implementing" }, {
      core.state_marker(event.proposal_id, "implementing", state_version),
    })

    local implementing_result = run_reconcile(event, opts("reconcile-terminal-implementing"))
    t.eq(implementing_result.exit_code, 0)
    t.eq(#implementing_result.raises, 0)
  end,

  test_review_reconcile_blocks_when_live_version_outranks_issue_version = function()
    local event = review_reconcile()
    local state_version = event.issue_version .. "/review-loop/9"
    mock_bot_env()
    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", state_version),
    })

    local result = run_review_reconcile(event, opts("reconcile-terminal-reviewing"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment = find_raise(result.raises, "github-proxy.github_pr_comment_request").payload
    local version = core.review_reconcile_terminal_state_version(state_version, event.round)
    t.eq(core.versioned_transition_status({ state = "reviewing", version = state_version }, { "reviewing" }, "blocked", version), "apply")
    t.is_true(comment.body:find(core.state_marker(event.proposal_id, "blocked", version), 1, true) ~= nil)
  end,

  test_review_reconcile_does_not_override_advanced_state = function()
    local event = review_reconcile()
    local state_version = core.review_reconcile_terminal_state_version(event.issue_version .. "/review-loop/9", event.round)
    mock_bot_env()
    mock_issue_review({ "fkst-dev:implementing" }, {
      core.state_marker(event.proposal_id, "implementing", state_version),
    })

    local implementing_result = run_review_reconcile(event, opts("reconcile-terminal-review-implementing"))
    t.eq(implementing_result.exit_code, 0)
    t.eq(#implementing_result.raises, 0)

    mock_bot_env()
    mock_issue_review({ "fkst-dev:merge-ready" }, {
      core.state_marker(event.proposal_id, "merge-ready", state_version),
    })

    local merge_ready_result = run_review_reconcile(event, opts("reconcile-terminal-review-merge-ready"))
    t.eq(merge_ready_result.exit_code, 0)
    t.eq(#merge_ready_result.raises, 0)
  end,

  test_timeout_reconcile_blocks_pr_local_state_on_pr_comment_stream = function()
    local issue_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/timeout/fixing/3"
    local payload = core.build_devloop_timeout_reconcile_payload(
      core.restart_transition_row("fixing"),
      {
        state = "fixing",
        version = issue_version,
        proposal_id = "github-devloop/issue/owner/repo/42",
      },
      "github-devloop/issue/owner/repo/42",
      core.pr_source_ref("owner/repo", 7),
      3
    )
    mock_bot_env()
    mock_issue_review({ "fkst-dev:fixing" }, {
      core.state_marker(payload.proposal_id, "fixing", issue_version),
    })
    entity_read_mocks.mock_default_pr_read(t, {
      core.state_marker(payload.proposal_id, "fixing", issue_version),
    })
    h.mock_default_issue_claim("owner/repo", 42)

    local result = run_timeout_reconcile(payload, opts("timeout-reconcile-pr-local"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local comment = find_raise(result.raises, "github-proxy.github_pr_comment_request").payload
    local label = find_raise(result.raises, "github-proxy.github_issue_label_request").payload
    local version = core.timeout_reconcile_state_version(issue_version, "fixing", 3)
    t.eq(tonumber(comment.pr_number), 7)
    t.is_true(comment.body:find(core.state_marker(payload.proposal_id, "blocked", version), 1, true) ~= nil)
    t.is_true(comment.body:find("fkst:github-devloop:timeout-reconcile:v1", 1, true) ~= nil)
    t.is_true(comment.body:find("state-output-obligation-timeout-after-3-attempts", 1, true) ~= nil)
    t.eq(tonumber(label.target_number), 42)
    t.eq(label.add_labels[1], "fkst-dev:blocked")
  end,
}
