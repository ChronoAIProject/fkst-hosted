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

local function angles(round, verdict)
  return {
    { angle = "minimal", verdict = verdict or "abstain", digest = "digest-" .. tostring(round or 0) },
  }
end

local function cap_angles(round)
  local verdicts = { "abstain", "comment", "approve" }
  return angles(round, verdicts[(round % #verdicts) + 1])
end

local function findings(text)
  return "open:\n" .. tostring(text or "current unresolved finding")
end

local function without_generation(marker)
  local body, replaced = tostring(marker):gsub(' generation="[^"]*"', "")
  if replaced ~= 1 then
    error("github-devloop test fixture expected one generation attribute")
  end
  return body
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

  test_loop_uses_proposal_lineage_when_version_and_source_ref_drift = function()
    local base_version = "consensus:github-devloop/issue/owner/repo/42/intake/current"
    local drift_version = "consensus:github-devloop/issue/owner/repo/42/intake/drifted"
    local event = unresolved({
      dedup_key = base_version .. "/loop/3",
      round = 3,
      source_ref = { kind = "external", ref = "owner/repo#issue/42?current=1" },
      narrowed_question = "Current boundary question",
      angle_digests = angles(0),
    })
    local current_digest = convergence_shared.source_ref_digest(event.source_ref)
    local drift_digest = convergence_shared.source_ref_digest({ kind = "external", ref = "owner/repo#issue/42?drift=1" })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.state_marker(event.proposal_id, "thinking", base_version),
      {
        body = conv_rounds.converge_round_marker(event.proposal_id, base_version, current_digest, 1, base_version .. "/loop/1", "Forged", angles(1), findings("forged finding")),
        author_login = "ordinary-user",
      },
      conv_rounds.converge_round_marker(event.proposal_id, drift_version, drift_digest, 2, drift_version .. "/loop/2", "Other boundary", angles(2), findings("drifted finding"), false, base_version),
    })

    local result = run_loop(event, opts("loop-drifted-lineage-budget"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local proposal = find_raise(result.raises, "consensus.proposal")
    t.is_true(proposal ~= nil)
    t.eq(proposal.payload.dedup_key, "github-devloop/issue/owner/repo/42/intake/current/loop/4")
    t.eq(proposal.payload.round, 4)
    t.eq(proposal.payload.prior_round_digests, nil)
    t.eq(find_raise(result.raises, "devloop_reconcile"), nil)
  end,

  test_loop_stale_lower_round_does_not_reset_after_drifted_lineage = function()
    local base_version = "consensus:github-devloop/issue/owner/repo/42/intake/current"
    local drift_version = "consensus:github-devloop/issue/owner/repo/42/intake/drifted"
    local event = unresolved({
      dedup_key = base_version .. "/loop/1",
      round = 1,
      source_ref = { kind = "external", ref = "owner/repo#issue/42?current=1" },
      narrowed_question = "Stale lower round",
      angle_digests = angles(1),
    })
    local drift_digest = convergence_shared.source_ref_digest({ kind = "external", ref = "owner/repo#issue/42?drift=1" })
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.state_marker(event.proposal_id, "thinking", base_version),
      conv_rounds.converge_round_marker(event.proposal_id, drift_version, drift_digest, 2, drift_version .. "/loop/2", "Other boundary", angles(2), nil, false, base_version),
    })

    local result = run_loop(event, opts("loop-stale-lower-drifted-lineage"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_loop_round_cap_uses_proposal_lineage_across_drifting_boundaries = function()
    local base_version = "consensus:github-devloop/issue/owner/repo/42/intake/current"
    local event = unresolved({
      dedup_key = base_version .. "/loop/8",
      round = 8,
      source_ref = { kind = "external", ref = "owner/repo#issue/42?current=8" },
      narrowed_question = "Question 8 with new surface text",
      angle_digests = cap_angles(8),
    })
    local comments = {
      core.state_marker(event.proposal_id, "thinking", base_version),
    }
    for round = 1, 7 do
      local drift_version = "consensus:github-devloop/issue/owner/repo/42/intake/drifted-" .. tostring(round)
      local source_ref = { kind = "external", ref = "owner/repo#issue/42?drift=" .. tostring(round) }
      table.insert(comments, conv_rounds.converge_round_marker(event.proposal_id,
        drift_version,
        convergence_shared.source_ref_digest(source_ref),
        round,
        drift_version .. "/loop/" .. tostring(round),
        "Question " .. tostring(round),
        cap_angles(round),
        nil,
        false,
        base_version
      ))
    end
    mock_issue_loop({ "fkst-dev:thinking" }, comments)

    local result = run_loop(event, opts("loop-round-cap-drifted-lineage"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment ~= nil)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
    t.eq(comment.payload.handoff.kind, "github-devloop.reconcile")
    t.eq(comment.payload.handoff.round, 8)
    t.eq(comment.payload.handoff.base_version, base_version)
    t.is_true(comment.payload.body:find('round="8"', 1, true) ~= nil)
  end,

  test_loop_prior_generation_cap_does_not_terminalize_new_generation = function()
    local old_generation = "consensus:github-devloop/issue/owner/repo/42/intake/old"
    local new_generation = "consensus:github-devloop/issue/owner/repo/42/intake/new"
    local event = unresolved({
      dedup_key = new_generation,
      round = 0,
      narrowed_question = "New generation question",
      angle_digests = angles(0, "abstain"),
    })
    local comments = {
      core.state_marker(event.proposal_id, "blocked", old_generation .. "/loop/8"),
      core.state_marker(event.proposal_id, "thinking", new_generation),
    }
    for round = 0, 8 do
      table.insert(comments, conv_rounds.converge_round_marker(event.proposal_id,
        old_generation,
        convergence_shared.source_ref_digest({ kind = "external", ref = "owner/repo#issue/42?old=" .. tostring(round) }),
        round,
        old_generation .. "/loop/" .. tostring(round),
        "Old generation question " .. tostring(round),
        cap_angles(round),
        nil,
        false,
        old_generation
      ))
    end
    mock_issue_loop({ "fkst-dev:thinking" }, comments)

    local result = run_loop(event, opts("loop-prior-generation-cap-isolated"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local proposal = find_raise(result.raises, "consensus.proposal")
    t.is_true(proposal ~= nil)
    t.eq(proposal.payload.round, 1)
    t.eq(proposal.payload.dedup_key, "github-devloop/issue/owner/repo/42/intake/new/loop/1")
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment ~= nil)
    t.is_nil(comment.payload.handoff)
    t.is_true(comment.payload.body:find('round="0"', 1, true) ~= nil)
    t.is_true(comment.payload.body:find('generation="github-devloop/issue/owner/repo/42/intake/new"', 1, true) ~= nil)
  end,

  test_loop_legacy_missing_generation_budget_is_observable_and_does_not_drop = function()
    local current_generation = "consensus:github-devloop/issue/owner/repo/42/intake/current"
    local event = unresolved({
      dedup_key = current_generation,
      round = 0,
      source_ref = { kind = "external", ref = "owner/repo#issue/42?current=deploy" },
      narrowed_question = "Current generation starts after deploy",
      angle_digests = angles(0, "abstain"),
    })
    local comments = {
      core.state_marker(event.proposal_id, "thinking", current_generation),
    }
    for round = 1, 8 do
      local drift_version = "consensus:github-devloop/issue/owner/repo/42/intake/drifted-" .. tostring(round)
      local source_ref = { kind = "external", ref = "owner/repo#issue/42?legacy=" .. tostring(round) }
      table.insert(comments, without_generation(conv_rounds.converge_round_marker(event.proposal_id,
        drift_version,
        convergence_shared.source_ref_digest(source_ref),
        round,
        drift_version .. "/loop/" .. tostring(round),
        "Legacy question " .. tostring(round),
        cap_angles(round),
        nil,
        false,
        current_generation
      )))
    end
    mock_issue_loop({ "fkst-dev:thinking" }, comments)

    local current_lineage = conv_rounds.converge_round_facts_for_generation(comments, event.proposal_id, current_generation)
    local legacy_lineage = conv_rounds.legacy_converge_round_facts_without_generation(comments, event.proposal_id)
    t.eq(#current_lineage, 0)
    t.eq(conv_rounds.max_converge_round(legacy_lineage), 8)

    local result = run_loop(event, opts("loop-legacy-missing-generation-observable"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local proposal = find_raise(result.raises, "consensus.proposal")
    t.is_true(proposal ~= nil)
    t.eq(proposal.payload.round, 1)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment ~= nil)
    t.is_nil(comment.payload.handoff)
  end,

  test_loop_current_generation_terminates_with_current_base_despite_drifted_versions = function()
    local current_generation = "consensus:github-devloop/issue/owner/repo/42/intake/current"
    local event = unresolved({
      dedup_key = current_generation .. "/loop/9",
      round = 9,
      source_ref = { kind = "external", ref = "owner/repo#issue/42?current=9" },
      narrowed_question = "Current generation over cap",
      angle_digests = cap_angles(9),
    })
    local comments = {
      core.state_marker(event.proposal_id, "thinking", current_generation),
    }
    for round = 1, 8 do
      local legacy_version = "consensus:github-devloop/issue/owner/repo/42/intake/legacy-drifted-" .. tostring(round)
      local source_ref = { kind = "external", ref = "owner/repo#issue/42?legacy-terminal=" .. tostring(round) }
      table.insert(comments, without_generation(conv_rounds.converge_round_marker(event.proposal_id,
        legacy_version,
        convergence_shared.source_ref_digest(source_ref),
        round,
        legacy_version .. "/loop/" .. tostring(round),
        "Legacy question " .. tostring(round),
        cap_angles(round),
        nil,
        false,
        current_generation
      )))
    end
    for round = 0, 8 do
      local drift_version = "consensus:github-devloop/issue/owner/repo/42/intake/current-drifted-" .. tostring(round)
      local source_ref = { kind = "external", ref = "owner/repo#issue/42?current-drift=" .. tostring(round) }
      table.insert(comments, conv_rounds.converge_round_marker(event.proposal_id,
        drift_version,
        convergence_shared.source_ref_digest(source_ref),
        round,
        drift_version .. "/loop/" .. tostring(round),
        "Current question " .. tostring(round),
        cap_angles(round),
        nil,
        false,
        current_generation
      ))
    end
    mock_issue_loop({ "fkst-dev:thinking" }, comments)

    local result = run_loop(event, opts("loop-current-generation-terminal-base"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
    local comment = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment ~= nil)
    t.eq(comment.payload.handoff.kind, "github-devloop.reconcile")
    t.eq(comment.payload.handoff.round, 8)
    t.eq(comment.payload.handoff.base_version, current_generation)
  end,

  test_loop_distinct_progressing_rounds_below_cap_continue = function()
    local base_version = "consensus:github-devloop/issue/owner/repo/42/intake/current"
    local event = unresolved({
      dedup_key = base_version .. "/loop/3",
      round = 3,
      narrowed_question = "Question 3",
      angle_digests = angles(3, "approve"),
    })
    local sr_digest = convergence_shared.source_ref_digest(event.source_ref)
    mock_issue_loop({ "fkst-dev:thinking" }, {
      core.state_marker(event.proposal_id, "thinking", base_version),
      conv_rounds.converge_round_marker(event.proposal_id, base_version, sr_digest, 1, base_version .. "/loop/1", "Question 1", angles(1, "abstain")),
      conv_rounds.converge_round_marker(event.proposal_id, base_version, sr_digest, 2, base_version .. "/loop/2", "Question 2", angles(2, "comment")),
    })

    local result = run_loop(event, opts("loop-distinct-progressing-continues"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 2)
    local proposal = find_raise(result.raises, "consensus.proposal")
    t.is_true(proposal ~= nil)
    t.eq(proposal.payload.round, 4)
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
