local entity_lib = require("devloop.entity")
local strings = require("contract.strings")
local m_claims = require("devloop.claims")
local parsers_misc = require("devloop.parsers.misc")
local core = require("core")
local saga = require("workflow.saga")
local source_refs = require("contract.source_ref")
local handoff_helpers = require("devloop.comment_handoff")
local base_ids = require("devloop.base_ids")
local devloop_base = require("devloop.base")

local payloads_builders = require("devloop.payloads.builders")
local payloads_predicates = require("devloop.payloads.predicates")
local spec = {
  consumes = { "github-proxy.github_comment_written" },
  produces = {
    "devloop_observe_pr",
    "devloop_merge_ready",
    "devloop_fixing",
    "devloop_reviewing",
    "github-proxy.github_issue_label_request",
  },
  fanout = { "github-proxy.github_comment_written" },
  stall_window = "30s",
}

local function valid_base_pr_handoff(handoff)
  return entity_lib.is_safe_entity_proposal_ref(handoff.proposal_id, handoff.version)
    and require("devloop.pr_safety").is_safe_pr_number(handoff.pr_number)
    and strings.is_bounded_string(handoff.version, core._max_dedup_len)
    and source_refs.has_bounded_source_ref(handoff.source_ref, core._max_key_len)
end

local function valid_review_handoff(handoff)
  return valid_base_pr_handoff(handoff)
    and devloop_base.is_safe_pr_review_result_ref(handoff.review_proposal_id, handoff.review_dedup_key)
    and require("devloop.pr_safety").is_safe_head_sha(handoff.reviewed_head_sha)
end

local function valid_merge_ready_handoff(handoff)
  return valid_review_handoff(handoff)
    and require("devloop.pr_safety").is_safe_head_sha(handoff.current_head_sha)
end

local function valid_fixing_handoff(handoff)
  return valid_review_handoff(handoff)
    and (handoff.current_head_sha == nil or require("devloop.pr_safety").is_safe_head_sha(handoff.current_head_sha))
    and (handoff.blocking_gap == nil or strings.is_bounded_string(handoff.blocking_gap, core._max_blocking_gap_len))
    and (handoff.framing == nil or strings.is_bounded_string(handoff.framing, core._max_framing_len))
    and (handoff.gate_baseline_sha == nil or require("devloop.pr_safety").is_safe_head_sha(handoff.gate_baseline_sha))
    and (handoff.gate_failure_excerpt == nil or strings.is_bounded_string(handoff.gate_failure_excerpt, parsers_misc.max_rollup_failure_summary_len))
    and (handoff.predecessor_set == nil or strings.is_path_safe_key(handoff.predecessor_set, core._max_dedup_len))
    and (handoff.dedup_key == nil or strings.is_path_safe_key(handoff.dedup_key, core._max_dedup_len))
end

local function issue_claim_ok(payload, handoff)
  local entity = entity_lib.parse_entity_proposal_id(handoff.proposal_id)
  if entity == nil then
    return false
  end
  if entity.kind == "pr" then
    local repo = payload.repo
    if repo == nil then
      repo = select(1, devloop_base.parse_pr_source_ref(handoff.source_ref))
    end
    return entity.repo == repo and tostring(entity.pr_number) == tostring(handoff.pr_number)
  end
  return m_claims.verify_pr_review_issue_claim(core, "comment_handoff", entity.repo, entity.issue_number, nil, handoff.proposal_id)
end

local function verified_pr_state(repo, handoff, comment_id, state)
  local expected = {
    proposal_id = handoff.proposal_id,
    state = state,
    marker_version = handoff.version,
    event_version = handoff.version,
  }
  local marker_hand_off = {
    kind = "own-state-marker",
    proposal_id = handoff.proposal_id,
    state = state,
    marker_version = handoff.version,
    event_version = handoff.version,
    stage_rank = core.stage_rank(state),
    comment_id = comment_id,
  }
  return payloads_predicates.verified_hand_off_state(core, repo, marker_hand_off, expected)
end

local function retryable_visibility_reason(reason)
  return reason == "state-marker-missing" or reason == "comment-get-failed"
end

local maybe_raise_pr_label

local function emit_merge_ready(payload, handoff)
  local merge_ready = payloads_builders.build_devloop_merge_ready_payload(core, handoff.proposal_id, handoff.pr_number, handoff.version, {
    review_proposal_id = handoff.review_proposal_id,
    review_dedup_key = handoff.review_dedup_key,
    reviewed_head_sha = handoff.reviewed_head_sha,
    current_head_sha = handoff.current_head_sha,
  }, handoff.source_ref)
  core.log_cas_decision("comment_handoff", handoff.proposal_id, { state = "merge-ready", version = handoff.version }, "comment-written", "devloop_merge_ready", "applied(own-write-comment-id)", "merge-ready marker comment write was acknowledged")
  core.log_raise("comment_handoff", handoff.proposal_id, "devloop_merge_ready", merge_ready)
  maybe_raise_pr_label(payload, handoff)
end

local function emit_fixing(payload, handoff)
  local fixing = payloads_builders.build_devloop_fixing_payload(core, {
    proposal_id = handoff.proposal_id,
    impl_version = handoff.version,
  }, handoff.pr_number, {
    review_proposal_id = handoff.review_proposal_id,
    review_dedup_key = handoff.review_dedup_key,
    reviewed_head_sha = handoff.reviewed_head_sha,
    framing = handoff.framing,
    blocking_gap = handoff.blocking_gap,
    gate_baseline_sha = handoff.gate_baseline_sha,
    predecessor_set = handoff.predecessor_set,
    gate_failure_excerpt = handoff.gate_failure_excerpt,
  }, handoff.source_ref)
  if handoff.dedup_key ~= nil then
    fixing.dedup_key = handoff.dedup_key
  end
  core.log_cas_decision("comment_handoff", handoff.proposal_id, { state = "fixing", version = handoff.version }, "comment-written", "devloop_fixing", "applied(own-write-comment-id)", "fixing marker comment write was acknowledged")
  core.log_raise("comment_handoff", handoff.proposal_id, "devloop_fixing", fixing)
  maybe_raise_pr_label(payload, handoff)
end

local function emit_claimed_label(payload, handoff)
  if not issue_claim_ok(payload, handoff) then
    return
  end
  maybe_raise_pr_label(payload, handoff)
end

local function emit_pr_open(payload, handoff)
  local repo = payload.repo
  if repo == nil then
    repo = select(1, devloop_base.parse_pr_source_ref(handoff.source_ref))
  end
  if repo == nil then
    error("comment-handoff: pr-open-missing-repo: PR-open handoff missing repo")
  end
  local verified_state, reason = verified_pr_state(repo, handoff, payload.comment_id, "pr-open")
  if verified_state == nil then
    if retryable_visibility_reason(reason) then
      core.log_cas_decision("comment_handoff", handoff.proposal_id, { state = nil, version = nil }, "comment-written", "devloop_observe_pr", "retry-pending(pr-open marker not visible)", "pr-open marker comment write was acknowledged but exact marker is not visible")
      error("comment-handoff: pr-open-marker-not-visible: pr-open marker not visible for PR observer handoff; retrying")
    end
    core.log_cas_decision("comment_handoff", handoff.proposal_id, { state = nil, version = nil }, "comment-written", "devloop_observe_pr", "skip-stale(" .. tostring(reason) .. ")", "state marker handoff no longer matches PR-open observer precondition")
    return
  end

  local observe = {
    schema = "github-proxy.v1",
    type = "pr",
    repo = repo,
    number = tonumber(handoff.pr_number),
    state = "OPEN",
    updated_at = payload.updated_at or "",
    source = "comment-handoff",
    dedup_key = base_ids.dedup_key({
      "comment-handoff",
      "pr-open",
      tostring(handoff.proposal_id),
      tostring(handoff.version),
      tostring(handoff.pr_number),
      tostring(payload.comment_id),
    }),
    source_ref = base_ids.normalize_source_ref(handoff.source_ref),
  }
  core.log_cas_decision("comment_handoff", handoff.proposal_id, verified_state, "comment-written", "devloop_observe_pr", "applied(own-write-comment-id)", "pr-open marker comment write was acknowledged")
  core.log_raise("comment_handoff", handoff.proposal_id, "devloop_observe_pr", observe)
end

local function emit_reviewing(payload, handoff)
  if not issue_claim_ok(payload, handoff) then
    return
  end
  local reviewing = payloads_builders.build_devloop_reviewing_payload(core, {
    proposal_id = handoff.proposal_id,
    impl_version = handoff.version,
    reviewing_comment_id = payload.comment_id,
  }, handoff.pr_number, handoff.source_ref, handoff.version)
  core.log_cas_decision("comment_handoff", handoff.proposal_id, { state = "reviewing", version = handoff.version }, "comment-written", "devloop_reviewing", "applied(own-write-comment-id)", "reviewing marker comment write was acknowledged")
  core.log_raise("comment_handoff", handoff.proposal_id, "devloop_reviewing", reviewing)
  maybe_raise_pr_label(payload, handoff)
end

local handoff_strategies = {
  ["github-devloop.pr_open"] = {
    validate = valid_base_pr_handoff,
    state = "pr-open",
    emit = emit_pr_open,
  },
  ["github-devloop.reviewing"] = {
    validate = valid_base_pr_handoff,
    state = "reviewing",
    emit = emit_reviewing,
  },
  ["github-devloop.blocked"] = {
    validate = valid_base_pr_handoff,
    state = "blocked",
    emit = emit_claimed_label,
  },
  ["github-devloop.closed_unmerged"] = {
    validate = valid_base_pr_handoff,
    state = "closed-unmerged",
    emit = emit_claimed_label,
  },
  ["github-devloop.merge_ready"] = {
    validate = valid_merge_ready_handoff,
    state = "merge-ready",
    emit = emit_merge_ready,
  },
  ["github-devloop.fixing"] = {
    validate = valid_fixing_handoff,
    state = "fixing",
    emit = emit_fixing,
  },
}

local function supported_handoff(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-proxy.comment-written.v1"
    or not payloads_predicates.is_safe_comment_id(core, payload.comment_id)
    or type(payload.handoff) ~= "table" then
    return nil
  end
  local handoff = payload.handoff
  local strategy = handoff_strategies[handoff.kind]
  if strategy ~= nil and strategy.validate(handoff) then
    return handoff
  end
  return nil
end

local accept_handoff = handoff_helpers.acceptor(supported_handoff)

local function handoff_done(_event)
  return false
end

local log_pr_unsupported_handoff = function(event) return handoff_helpers.log_unsupported(core, supported_handoff, event) end

local function handoff_state(handoff)
  local strategy = handoff_strategies[handoff.kind]
  if strategy == nil then
    return nil
  end
  return strategy.state
end

local function issue_number_for_label(payload, handoff, repo)
  if payload.issue_number ~= nil then
    return payload.issue_number
  end
  local entity = entity_lib.parse_entity_proposal_id(handoff.proposal_id)
  if entity ~= nil and entity.kind == "issue" and entity.repo == repo then
    return entity.issue_number
  end
  return nil
end

maybe_raise_pr_label = function(payload, handoff)
  local state = handoff_state(handoff)
  if state == nil then
    return
  end
  local repo = payload.repo
  if repo == nil then
    repo = select(1, devloop_base.parse_pr_source_ref(handoff.source_ref))
  end
  if repo == nil then
    error("github-devloop: PR label handoff missing repo")
  end
  local verified_state, reason = verified_pr_state(repo, handoff, payload.comment_id, state)
  if verified_state == nil then
    if retryable_visibility_reason(reason) then
      core.log_cas_decision("comment_handoff", handoff.proposal_id, { state = nil, version = nil }, "comment-written", "github-proxy.github_issue_label_request", "retry-pending(" .. tostring(state) .. " marker not visible)", tostring(state) .. " marker comment write was acknowledged but exact marker is not visible")
      error("github-devloop: " .. tostring(state) .. " marker not visible for PR label handoff; retrying")
    end
    core.log_cas_decision("comment_handoff", handoff.proposal_id, { state = nil, version = nil }, "comment-written", "github-proxy.github_issue_label_request", "skip-stale(" .. tostring(reason) .. ")", "state marker handoff no longer matches PR label precondition")
    return
  end

  local label_request = core.build_reconcile_pr_state_label_request(
    repo,
    issue_number_for_label(payload, handoff, repo),
    handoff.pr_number,
    handoff.proposal_id,
    verified_state.state,
    verified_state.version,
    handoff.source_ref
  )
  core.log_apply("comment_handoff", handoff.proposal_id, verified_state.state, verified_state.version, { add = label_request.add_labels, remove = label_request.remove_labels }, {
    "github-proxy.github_issue_label_request",
  })
  core.log_raise("comment_handoff", handoff.proposal_id, "github-proxy.github_issue_label_request", label_request)
end

local function act_handoff(event)
  local payload = event.payload or {}
  local handoff = supported_handoff(payload)
  if handoff == nil then
    log_pr_unsupported_handoff(event)
    return
  end

  core.log_entry("comment_handoff", event, handoff.proposal_id, payload.dedup_key)
  handoff_strategies[handoff.kind].emit(payload, handoff)
end

return saga.department(spec, {
  accept = accept_handoff,
  done = handoff_done,
  act = act_handoff,
  on_skip_foreign = log_pr_unsupported_handoff,
  wrap = core.wrap_pipeline_failure,
  name = "comment_handoff",
})
