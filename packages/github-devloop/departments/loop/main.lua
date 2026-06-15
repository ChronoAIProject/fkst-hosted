local core = require("core")

local M = {}

M.spec = {
  consumes = { "consensus.consensus_converge" },
  produces = {
    "devloop_reconcile",
    "consensus.proposal",
    "github-proxy.github_issue_comment_request",
  },
  fanout = { "consensus.consensus_converge" },
  stall_window = "30s",
}

local function append_round_fact(facts, round, narrowed_question, angle_digests, dedup_key)
  local copied = {}
  for _, fact in ipairs(facts or {}) do
    table.insert(copied, fact)
  end
  table.insert(copied, {
    round = round,
    question = core.converge_question_digest(narrowed_question),
    verdicts = core.converge_verdicts_digest(angle_digests),
    dedup = dedup_key,
  })
  return copied
end

function pipeline(event)
  local unresolved = event.payload or {}
  if not core.is_supported_unresolved(unresolved) then
    core.log_entry("loop", event, "unknown", core.payload_field(unresolved, "dedup_key"))
    core.log_cas_decision("loop", "unknown", { state = nil, version = nil }, "thinking", "thinking", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("loop", event, unresolved.proposal_id, unresolved.dedup_key)
  local repo, issue_number = core.parse_proposal_id(unresolved.proposal_id)
  if repo == nil then
    core.log_cas_decision("loop", unresolved.proposal_id, { state = nil, version = nil }, "thinking", "thinking", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end

  local lock_key = core.loop_lock_key(unresolved.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("loop", unresolved.proposal_id, { state = nil, version = nil }, "thinking", "thinking", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local view = core.gh_exec({ cmd = core.gh_issue_view_loop_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue loop view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_loop(view.stdout)
    core.log_forged_markers("loop", unresolved.proposal_id, current.comments)
    local state = core.current_state(current.comments, unresolved.proposal_id)
    local transition = core.transition_status(state, { "thinking" }, "blocked")
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "thinking", core.cas_outcome(state, transition, unresolved.dedup_key), "unresolved event cannot advance current marker")
      return
    end
    if transition == "pending" then
      core.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "thinking", core.cas_outcome(state, transition, unresolved.dedup_key), "thinking state marker not yet visible")
      error("github-devloop: thinking state marker not yet visible for unresolved; retrying")
    end

    local base_version = core.converge_base_version(unresolved.dedup_key)
    local sr_digest = core.source_ref_digest(unresolved.source_ref)
    local facts = core.converge_round_facts(current.comments, unresolved.proposal_id, base_version, sr_digest)
    local round = math.max(tonumber(unresolved.round) or 0, core.max_converge_round(facts))
    if core.has_converge_round_marker(current.comments, unresolved.proposal_id, base_version, sr_digest, round) then
      core.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "thinking", "skip-idempotent(converge round marker already visible)", "converge round marker for incoming round is already visible")
      return
    end

    local marker_body = core.converge_round_marker(
      unresolved.proposal_id,
      base_version,
      sr_digest,
      round,
      unresolved.dedup_key,
      unresolved.narrowed_question,
      unresolved.angle_digests
    )
    local comment_request = core.build_converge_round_comment_request(repo, issue_number, unresolved, round, marker_body)
    local facts_with_current = append_round_fact(facts, round, unresolved.narrowed_question, unresolved.angle_digests, unresolved.dedup_key)
    local budget_round = math.max(round, core.converge_budget_round(current.comments, unresolved.proposal_id))
    local hit_round_cap = budget_round >= core.max_converge_rounds()
    if hit_round_cap or core.is_true_stall(facts_with_current, round) then
      local reconcile = core.build_devloop_reconcile_payload(unresolved, round, base_version)
      local reason = hit_round_cap
        and ("convergence budget reached at round " .. tostring(budget_round))
        or ("true convergence stall at round " .. tostring(round))
      core.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "thinking", core.cas_outcome(state, transition, unresolved.dedup_key), reason)
      core.log_apply("loop", unresolved.proposal_id, nil, nil, { add = {}, remove = {} }, {
        "github-proxy.github_issue_comment_request",
        "devloop_reconcile",
      })
      core.log_raise("loop", unresolved.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
      core.log_raise("loop", unresolved.proposal_id, "devloop_reconcile", reconcile)
      return
    end

    local next_n = round + 1
    local next_dedup = core.converge_proposal_base_dedup(unresolved.dedup_key) .. "/loop/" .. tostring(next_n)
    local content_fetch = core.context_fetch_ref_from_bundle({
      dept = "loop",
      repo = repo,
      issue_number = issue_number,
      proposal_id = unresolved.proposal_id,
      version = next_dedup,
      tick = event.ts,
    })
    local proposal = core.build_board_loop_proposal(repo, issue_number, current, unresolved.source_ref, next_n, {
      narrowed_question = unresolved.narrowed_question,
      angle_digests = unresolved.angle_digests,
    }, event.ts, content_fetch, next_dedup)
    if not core.validate_proposal(proposal) then
      log.warn("github-devloop dept=loop proposal_id=" .. tostring(unresolved.proposal_id) .. " tag=SKIP reason=cannot-build-valid-loop-proposal")
      return
    end

    core.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "thinking", core.cas_outcome(state, transition, unresolved.dedup_key), "raising loop proposal round " .. tostring(next_n))
    core.log_apply("loop", unresolved.proposal_id, nil, nil, { add = {}, remove = {} }, {
      "consensus.proposal",
      "github-proxy.github_issue_comment_request",
    })
    core.log_raise("loop", unresolved.proposal_id, "consensus.proposal", proposal)
    core.log_raise("loop", unresolved.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  end)
end

pipeline = core.wrap_pipeline_failure("loop", pipeline)

return M
