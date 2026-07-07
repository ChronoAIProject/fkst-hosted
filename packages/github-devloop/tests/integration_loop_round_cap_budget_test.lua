local convergence_shared = require("devloop.convergence.shared")
local h = require("tests.devloop_helpers")
local conv_rounds = require("devloop.convergence.rounds")
local conv_reconcile = require("devloop.convergence.reconcile")
local t = h.t
local core = h.core
local opts = h.opts
local unresolved = h.unresolved
local run_loop = h.run_loop
local mock_issue_loop = h.mock_issue_loop
local find_raise = h.find_raise

local function angles(round)
  return {
    { angle = "minimal", verdict = "abstain", digest = "digest-" .. tostring(round or 0) },
  }
end

local function findings(text)
  return "open:\n" .. tostring(text or "current unresolved finding")
end

local function run_comment_handoff_from_request(request, comment_id, name)
  return t.run_department("departments/comment_handoff/main.lua", {
    queue = "github-proxy.github_comment_written",
    payload = {
      schema = "github-proxy.comment-written.v1",
      repo = request.repo,
      target = "issue",
      issue_number = request.issue_number,
      comment_id = comment_id,
      request_dedup_key = request.dedup_key,
      dedup_key = tostring(request.dedup_key) .. "/written/" .. tostring(comment_id),
      source_ref = request.source_ref,
      handoff = request.handoff,
    },
  }, opts(name))
end

return {
  test_loop_first_resolvable_findings_converge_raises_one_memory_proposal = function()
    local base_version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local event = unresolved({
      dedup_key = base_version,
      round = 0,
      narrowed_question = "Which dependency evidence resolves the gap?",
      angle_digests = angles(0),
      findings_record = findings("dependency evidence remains unresolved"),
    })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.state_marker(event.proposal_id, "thinking", base_version),
    })

    local result = run_loop(event, opts("loop-first-resolvable-findings"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local proposal = find_raise(result.raises, "consensus.proposal")
    t.is_true(proposal ~= nil)
    t.eq(proposal.payload.round, 1)
    t.eq(proposal.payload.dedup_key, "github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/1")
    t.eq(proposal.payload.convergence_question, event.narrowed_question)
    t.eq(proposal.payload.findings_record, event.findings_record)
    t.eq(proposal.payload.prior_round_digests, nil)

    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment ~= nil)
    t.is_nil(comment.payload.handoff)
    t.is_true(comment.payload.body:find('round="0"', 1, true) ~= nil)
    t.is_true(comment.payload.body:find('findings_record="open:%0Adependency evidence remains unresolved"', 1, true) ~= nil)
  end,

  test_loop_second_resolvable_findings_converge_handoffs_reconcile = function()
    local base_version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local event = unresolved({
      dedup_key = base_version .. "/loop/1",
      round = 1,
      narrowed_question = "Which dependency evidence resolves the gap now?",
      angle_digests = angles(1),
      findings_record = findings("second resolvable finding"),
    })
    local sr_digest = convergence_shared.source_ref_digest(event.source_ref)
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.state_marker(event.proposal_id, "thinking", base_version),
      conv_rounds.converge_round_marker(event.proposal_id, base_version, sr_digest, 0, base_version, "First boundary", angles(0), findings("first resolvable finding")),
    })

    local result = run_loop(event, opts("loop-second-resolvable-findings"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment ~= nil)
    t.eq(comment.payload.handoff.kind, "github-devloop.reconcile")
    t.eq(comment.payload.handoff.proposal_id, event.proposal_id)
    t.eq(comment.payload.handoff.round, 1)
    t.eq(comment.payload.handoff.base_version, base_version)
    t.eq(comment.payload.handoff.source_ref.ref, event.source_ref.ref)

    local handoff = run_comment_handoff_from_request(
      comment.payload,
      "IC_resolvability_reconcile",
      "loop-resolvability-comment-handoff-reconcile"
    )
    t.eq(handoff.exit_code, 0)
    local reconcile_raise = find_raise(handoff.raises, "devloop_reconcile")
    t.is_true(reconcile_raise ~= nil)
    local expected = conv_reconcile.build_devloop_reconcile_payload(event, 1, base_version)
    t.eq(reconcile_raise.payload.schema, expected.schema)
    t.eq(reconcile_raise.payload.proposal_id, expected.proposal_id)
    t.eq(reconcile_raise.payload.dedup_key, expected.dedup_key)
    t.eq(reconcile_raise.payload.round, expected.round)
    t.eq(reconcile_raise.payload.base_version, expected.base_version)
  end,

  test_loop_drifted_or_untrusted_findings_markers_do_not_spend_current_boundary = function()
    local base_version = "consensus:github-devloop/issue/owner/repo/42/intake/current"
    local drift_version = "consensus:github-devloop/issue/owner/repo/42/intake/drifted"
    local event = unresolved({
      dedup_key = base_version,
      round = 0,
      source_ref = { kind = "external", ref = "owner/repo#issue/42?current=1" },
      narrowed_question = "Current boundary question",
      angle_digests = angles(0),
      findings_record = findings("current boundary finding"),
    })
    local current_digest = convergence_shared.source_ref_digest(event.source_ref)
    local drift_digest = convergence_shared.source_ref_digest({ kind = "external", ref = "owner/repo#issue/42?drift=1" })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.state_marker(event.proposal_id, "thinking", base_version),
      {
        body = conv_rounds.converge_round_marker(event.proposal_id, base_version, current_digest, 1, base_version .. "/loop/1", "Forged", angles(1), findings("forged finding")),
        author_login = "ordinary-user",
      },
      conv_rounds.converge_round_marker(event.proposal_id, drift_version, drift_digest, 2, drift_version .. "/loop/2", "Other boundary", angles(2), findings("drifted finding")),
    })

    local result = run_loop(event, opts("loop-drifted-untrusted-findings"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local proposal = find_raise(result.raises, "consensus.proposal")
    t.is_true(proposal ~= nil)
    t.eq(proposal.payload.dedup_key, "github-devloop/issue/owner/repo/42/intake/current/loop/1")
    t.eq(proposal.payload.findings_record, event.findings_record)
    t.eq(proposal.payload.prior_round_digests, nil)
    t.eq(find_raise(result.raises, "devloop_reconcile"), nil)
  end,

  test_loop_essence_stall_handoffs_terminal_reconcile_without_continuation = function()
    local base_version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local event = unresolved({
      dedup_key = base_version,
      round = 0,
      narrowed_question = "essence-stall + no source-verifiable evidence remains",
      angle_digests = angles(0),
      findings_record = findings("no source-verifiable evidence remains"),
      essence_stall = true,
    })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.state_marker(event.proposal_id, "thinking", base_version),
    })

    local result = run_loop(event, opts("loop-essence-stall"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment ~= nil)
    t.eq(comment.payload.handoff.kind, "github-devloop.reconcile")
    t.eq(comment.payload.handoff.round, 0)
    t.is_true(comment.payload.body:find('essence_stall="true"', 1, true) ~= nil)
  end,
}
