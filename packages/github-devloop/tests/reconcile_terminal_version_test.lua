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

}
