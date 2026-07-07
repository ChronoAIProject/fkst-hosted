local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local requests_lifecycle = require("devloop.requests.lifecycle")
local parsers_issue = require("devloop.parsers.issue")
local convergence_shared = require("devloop.convergence.shared")
local core, saga = require("core"), require("workflow.saga")
local context_bundle = require("devloop.context_bundle")
local config = require("devloop.config")



local payloads_builders = require("devloop.payloads.builders")
local conv_rounds = require("devloop.convergence.rounds")
local valid_round = require("devloop.rounds").valid_round
local v_unresolved = require("devloop.validators.unresolved")
local v_validate_proposal = require("devloop.validators.validate_proposal")
local entity_lib = require("devloop.entity")
local devloop_logging = require("devloop.logging")
local devloop_state = require("devloop.state")
local devloop_commands = require("devloop.commands")
local transition_version = require("contract.transition_version")
local spec = {
  consumes = { "consensus.consensus_converge" },
  produces = {
    "consensus.proposal",
    "github-proxy.github_issue_comment_request",
  },
  fanout = { "consensus.consensus_converge" },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local function latest_lineage_fact(lineage)
  local latest = nil
  for _, fact in ipairs(lineage or {}) do
    if type(fact) == "table" and (latest == nil or tonumber(fact.round) > tonumber(latest.round)) then
      latest = fact
    end
  end
  return latest
end

return saga.department(spec, { done = function() return false end, act = function(event)
  local unresolved = event.payload or {}
  if not v_unresolved.is_supported_unresolved(unresolved) then
    devloop_logging.log_entry("loop", event, "unknown", devloop_logging.payload_field(unresolved, "dedup_key"))
    devloop_logging.log_cas_decision("loop", "unknown", { state = nil, version = nil }, "thinking", "thinking", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  devloop_logging.log_entry("loop", event, unresolved.proposal_id, unresolved.dedup_key)
  local repo, issue_number = base_ids.parse_proposal_id(unresolved.proposal_id)
  if repo == nil then
    devloop_logging.log_cas_decision("loop", unresolved.proposal_id, { state = nil, version = nil }, "thinking", "thinking", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end

  local lock_key = entity_lib.loop_lock_key(unresolved.proposal_id)
  if lock_key == nil then
    devloop_logging.log_cas_decision("loop", unresolved.proposal_id, { state = nil, version = nil }, "thinking", "thinking", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    devloop_base.assert_trusted_bot_configured()

    local view = devloop_commands.gh_issue_view_loop(repo, issue_number, 30)
    if view.exit_code ~= 0 then
      error("github-devloop: issue-read-failed: gh issue loop view failed: " .. tostring(view.stderr))
    end

    local current = parsers_issue.parse_issue_view_loop(core, view.stdout)
    devloop_logging.log_forged_markers("loop", unresolved.proposal_id, current.comments)
    local state = devloop_state.current_state(current.comments, unresolved.proposal_id)
    local transition = devloop_state.transition_status(state, { "thinking" }, "blocked")
    if transition == "idempotent" or transition == "stale" then
      devloop_logging.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "thinking", devloop_state.cas_outcome(state, transition, unresolved.dedup_key), "unresolved event cannot advance current marker")
      return
    end
    if transition == "pending" then
      devloop_logging.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "thinking", devloop_state.cas_outcome(state, transition, unresolved.dedup_key), "thinking state marker not yet visible")
      error("github-devloop: state-marker-pending: thinking state marker not yet visible for unresolved; retrying")
    end

    local base_version = conv_rounds.converge_base_version(unresolved.dedup_key)
    local sr_digest = convergence_shared.source_ref_digest(unresolved.source_ref)
    local function build_comment_request(unresolved_for_comment, round_for_comment, marker_body_for_comment, handoff_for_comment)
      return requests_lifecycle.build_converge_round_comment_request(core, repo, issue_number, unresolved_for_comment, round_for_comment, marker_body_for_comment, handoff_for_comment)
    end
    local lineage = conv_rounds.converge_round_facts_for_proposal(current.comments, unresolved.proposal_id)
    local has_lineage = #lineage > 0
    local latest_round = conv_rounds.max_converge_round(lineage)
    local latest_fact = latest_lineage_fact(lineage)
    local terminal_lineage = has_lineage
      and (latest_round >= config.max_converge_rounds()
        or conv_rounds.is_true_stall(lineage, latest_round)
        or conv_rounds.has_essence_stall(lineage)
        or conv_rounds.resolvability_exhausted(lineage))
    if terminal_lineage then
      local terminal_base_version = latest_fact and latest_fact.version or base_version
      local terminal_unresolved = {
        proposal_id = unresolved.proposal_id,
        dedup_key = (latest_fact and latest_fact.dedup) or unresolved.dedup_key,
        source_ref = unresolved.source_ref,
        narrowed_question = (latest_fact and latest_fact.narrowed_question) or unresolved.narrowed_question,
        angle_digests = (latest_fact and latest_fact.angle_digests) or unresolved.angle_digests,
      }
      local comment_request = build_comment_request(terminal_unresolved, latest_round, "", {
        kind = "github-devloop.reconcile",
        proposal_id = unresolved.proposal_id,
        round = latest_round,
        base_version = terminal_base_version,
        source_ref = base_ids.normalize_source_ref(unresolved.source_ref),
      })
      devloop_logging.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "thinking", devloop_state.cas_outcome(state, transition, unresolved.dedup_key), "convergence lineage terminal at round " .. tostring(latest_round))
      devloop_logging.log_apply("loop", unresolved.proposal_id, nil, nil, { add = {}, remove = {} }, {
        "github-proxy.github_issue_comment_request",
      })
      devloop_logging.log_raise("loop", unresolved.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
      return
    end

    local incoming_round = valid_round(unresolved.round) or 0
    if has_lineage and incoming_round <= latest_round then
      devloop_logging.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "thinking", "skip-stale(converge round lineage already advanced)", "incoming converge round is not newer than the proposal lineage")
      return
    end
    local expected_round = has_lineage and (latest_round + 1) or 0
    if incoming_round ~= expected_round then
      devloop_logging.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "thinking", "skip-stale(converge round gap)", "incoming converge round is not the next proposal lineage round")
      return
    end
    local round = incoming_round

    local marker_body = conv_rounds.converge_round_marker(unresolved.proposal_id,
      base_version,
      sr_digest,
      round,
      unresolved.dedup_key,
      unresolved.narrowed_question,
      unresolved.angle_digests,
      unresolved.findings_record,
      unresolved.essence_stall == true
    )
    local lineage_with_current = conv_rounds.append_converge_round_fact(lineage, round, unresolved.narrowed_question, unresolved.angle_digests, unresolved.dedup_key, unresolved.findings_record, unresolved.essence_stall == true)
    local cap_exhausted = conv_rounds.max_converge_round(lineage_with_current) >= config.max_converge_rounds()
    local resolvability_exhausted = conv_rounds.resolvability_exhausted(lineage_with_current)
    local essence_stall = conv_rounds.has_essence_stall(lineage_with_current)
    local true_stall = conv_rounds.is_true_stall(lineage_with_current, round)
    if essence_stall or resolvability_exhausted or cap_exhausted or true_stall then
      local comment_request = build_comment_request(unresolved, round, marker_body, {
        kind = "github-devloop.reconcile",
        proposal_id = unresolved.proposal_id,
        round = round,
        base_version = base_version,
        source_ref = base_ids.normalize_source_ref(unresolved.source_ref),
      })
      local reason = essence_stall
        and ("essence stall at round " .. tostring(round))
        or resolvability_exhausted
          and ("resolvability budget reached at round " .. tostring(round))
          or cap_exhausted
            and ("convergence round cap reached at round " .. tostring(round))
            or ("true convergence stall at round " .. tostring(round))
      devloop_logging.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "thinking", devloop_state.cas_outcome(state, transition, unresolved.dedup_key), reason)
      devloop_logging.log_apply("loop", unresolved.proposal_id, nil, nil, { add = {}, remove = {} }, {
        "github-proxy.github_issue_comment_request",
      })
      devloop_logging.log_raise("loop", unresolved.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
      return
    end

    local next_n = round + 1
    local next_dedup = transition_version.loop_at(conv_rounds.converge_proposal_base_dedup(unresolved.dedup_key), next_n)
    local content_fetch = context_bundle.context_fetch_ref_from_bundle(core, {
      dept = "loop",
      repo = repo,
      issue_number = issue_number,
      proposal_id = unresolved.proposal_id,
      version = next_dedup,
      tick = event.ts,
    })
    local proposal = payloads_builders.build_board_loop_proposal(core, repo, issue_number, current, unresolved.source_ref, next_n, {
      narrowed_question = unresolved.narrowed_question,
      angle_digests = unresolved.angle_digests,
      findings_record = lineage_with_current[#lineage_with_current] and lineage_with_current[#lineage_with_current].findings_record,
    }, event.ts, content_fetch, next_dedup)
    if not v_validate_proposal.validate_proposal(proposal) then
      log.warn("github-devloop dept=loop proposal_id=" .. tostring(unresolved.proposal_id) .. " tag=SKIP reason=cannot-build-valid-loop-proposal")
      return
    end
    local comment_request = build_comment_request(unresolved, round, marker_body)

    devloop_logging.log_cas_decision("loop", unresolved.proposal_id, state, "thinking", "thinking", devloop_state.cas_outcome(state, transition, unresolved.dedup_key), "raising loop proposal round " .. tostring(next_n))
    devloop_logging.log_apply("loop", unresolved.proposal_id, nil, nil, { add = {}, remove = {} }, {
      "consensus.proposal",
      "github-proxy.github_issue_comment_request",
    })
    devloop_logging.log_raise("loop", unresolved.proposal_id, "consensus.proposal", proposal)
    devloop_logging.log_raise("loop", unresolved.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  end)
end, wrap = devloop_logging.wrap_pipeline_failure, name = "loop" })
