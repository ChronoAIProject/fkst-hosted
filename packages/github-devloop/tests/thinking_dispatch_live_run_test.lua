local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local codex_status = require("tests.codex_status_helpers")
local convergence_shared = require("devloop.convergence.shared")
local conv_rounds = require("devloop.convergence.rounds")
local t = h.t
local core = h.core
local issue = h.issue
local opts = h.opts
local run_observe = h.run_observe
local run_loop = h.run_loop
local mock_issue_state = h.mock_issue_state
local mock_issue_loop = h.mock_issue_loop
local find_raise = h.find_raise

local function converge_round_comment(event, proposal_id, base_version, round, question, verdict)
  return {
    body = conv_rounds.converge_round_marker(proposal_id,
      base_version,
      convergence_shared.source_ref_digest(event.source_ref),
      round,
      base_version .. "/loop/" .. tostring(round),
      question or ("Question " .. tostring(round)),
      {
        { angle = "minimal", verdict = verdict or "abstain", digest = "round-digest-" .. tostring(round) },
      }),
    created_at = "2026-06-03T00:0" .. tostring(round) .. ":00Z",
  }
end

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

  test_thinking_redrive_resumes_latest_visible_converge_round_lineage_from_base_state = function()
    local event = issue()
    local original = payloads_builders.build_proposal(event)
    local base_version = "consensus:" .. original.dedup_key
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", {
      {
        body = core.state_marker(original.proposal_id, "thinking", original.dedup_key),
        created_at = "2026-06-03T00:00:00Z",
      },
      converge_round_comment(event, original.proposal_id, base_version, 0, "Latest visible question", "abstain"),
    })

    local result = run_observe(event, opts("thinking-replay-latest-converge-round", {
      now = "2026-06-03T02:00:00Z",
    }))
    t.eq(result.exit_code, 0)
    local proposal = find_raise(result.raises, "consensus.proposal")
    t.is_true(proposal ~= nil)
    t.eq(proposal.payload.dedup_key, original.dedup_key .. "/loop/1")
    t.eq(proposal.payload.round, 1)
    t.eq(proposal.payload.convergence_question, "Latest visible question")
    t.eq(proposal.payload.findings_record, nil)
    t.eq(proposal.payload.prior_round_digests, nil)

    local loop_event = {
      schema = "consensus.consensus_converge.v1",
      proposal_id = proposal.payload.proposal_id,
      round = proposal.payload.round,
      narrowed_question = "Evidence continuation still needs a handoff",
      angle_digests = {
        { angle = "minimal", verdict = "approve", digest = "round-digest-1" },
      },
      dedup_key = "consensus:" .. proposal.payload.dedup_key,
      source_ref = proposal.payload.source_ref,
    }
    mock_issue_loop({ "fkst-dev:enabled", "fkst-dev:thinking" }, {
      {
        body = core.state_marker(original.proposal_id, "thinking", original.dedup_key),
        created_at = "2026-06-03T00:00:00Z",
      },
      converge_round_comment(event, original.proposal_id, base_version, 0, "Latest visible question", "abstain"),
    })
    local loop = run_loop(loop_event, opts("thinking-replay-loop-regenerates-lost-handoff"))
    t.eq(loop.exit_code, 0)
    t.eq(find_raise(loop.raises, "consensus.proposal"), nil)
    local marker = find_raise(loop.raises, "github-proxy.github_issue_comment_request")
    t.is_true(marker ~= nil)
    t.is_true(marker.payload.body:find('round="1"', 1, true) ~= nil)
    t.eq(marker.payload.handoff.terminal_cause, "evidence-continuation-budget-exhausted")
  end,

  test_thinking_redrive_defers_when_latest_visible_converge_round_run_is_live = function()
    local event = issue()
    local original = payloads_builders.build_proposal(event)
    local base_version = "consensus:" .. original.dedup_key
    local version = original.dedup_key .. "/loop/1"
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", {
      {
        body = core.state_marker(original.proposal_id, "thinking", original.dedup_key),
        created_at = "2026-06-03T00:00:00Z",
      },
      converge_round_comment(event, original.proposal_id, base_version, 0),
    })

    local run_opts = opts("thinking-latest-converge-live-consensus-redrive", {
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
}
