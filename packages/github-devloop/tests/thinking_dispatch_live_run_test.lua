local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local codex_status = require("tests.codex_status_helpers")
local t = h.t
local core = h.core
local issue = h.issue
local opts = h.opts
local run_observe = h.run_observe
local mock_issue_state = h.mock_issue_state
local find_raise = h.find_raise

return {
  test_thinking_redrive_defers_when_matching_consensus_run_is_live = function()
    local event = issue()
    local original = payloads_builders.build_proposal(event)
    local version = original.dedup_key .. "/loop/1"
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", {
      {
        body = core.state_marker(original.proposal_id, "thinking", version),
        created_at = "2026-06-03T00:00:00Z",
      },
    })

    local run_opts = opts("thinking-live-consensus-redrive", {
      now = "2026-06-03T02:00:00Z",
    })
    codex_status.seed_role_codex_run(run_opts, "consensus", original.proposal_id, version, {
      started_at = "2026-06-03T00:30:00Z",
      started_at_ms = nil,
      timeout_seconds = 7200,
      lease_expires_at_ms = nil,
    })
    local result = run_observe(event, run_opts)
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
    t.eq(find_raise(result.raises, "github-proxy.github_issue_comment_request"), nil)
  end,

  test_thinking_redrive_without_live_run_reuses_current_lineage = function()
    local event = issue()
    local original = payloads_builders.build_proposal(event)
    local version = original.dedup_key .. "/loop/1"
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", {
      {
        body = core.state_marker(original.proposal_id, "thinking", version),
        created_at = "2026-06-03T00:00:00Z",
      },
    })

    local result = run_observe(event, opts("thinking-no-live-consensus-redrive", {
      now = "2026-06-03T02:00:00Z",
    }))
    t.eq(result.exit_code, 0)
    local proposal = find_raise(result.raises, "consensus.proposal")
    t.is_true(proposal ~= nil)
    t.eq(proposal.payload.dedup_key, version)
    t.eq(proposal.payload.round, 1)
    local attempt = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(attempt ~= nil)
    t.is_true(tostring(attempt.payload.body or ""):find('round="1"', 1, true) ~= nil)
  end,
}
