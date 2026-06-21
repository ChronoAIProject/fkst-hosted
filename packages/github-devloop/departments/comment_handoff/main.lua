local core = require("core")
local saga = require("std.saga")
local source_refs = require("std.source_ref")
local valid_round = require("std.devloop_rounds").valid_round

local spec = {
  consumes = { "github-proxy.github_comment_written" },
  produces = {
    "devloop_ready",
    "devloop_merge_ready",
    "devloop_fixing",
    "devloop_reconcile",
    "devloop_reviewing",
  },
  stall_window = "30s",
}

local function supported_handoff(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-proxy.comment-written.v1"
    or not core.is_safe_comment_id(payload.comment_id)
    or type(payload.handoff) ~= "table" then
    return nil
  end
  local handoff = payload.handoff
  if handoff.kind == "github-devloop.ready"
    and core.is_safe_consensus_result_ref(handoff.proposal_id, handoff.version)
    and core.is_safe_consensus_result_ref(handoff.proposal_id, handoff.marker_version)
    and core._is_bounded_string(handoff.version, core._max_dedup_len)
    and source_refs.has_bounded_source_ref(handoff.source_ref, core._max_key_len) then
    return handoff
  end
  if handoff.kind == "github-devloop.reviewing"
    and core.is_safe_entity_proposal_ref(handoff.proposal_id, handoff.version)
    and core.is_safe_pr_number(handoff.pr_number)
    and core._is_bounded_string(handoff.version, core._max_dedup_len)
    and source_refs.has_bounded_source_ref(handoff.source_ref, core._max_key_len) then
    return handoff
  end
  if handoff.kind == "github-devloop.reconcile"
    and core.is_safe_consensus_result_ref(handoff.proposal_id, handoff.base_version)
    and core._is_bounded_string(handoff.base_version, core._max_dedup_len)
    and valid_round(handoff.round) ~= nil
    and source_refs.has_bounded_source_ref(handoff.source_ref, core._max_key_len) then
    return handoff
  end
  if handoff.kind == "github-devloop.merge_ready"
    and core.is_safe_entity_proposal_ref(handoff.proposal_id, handoff.version)
    and core.is_safe_pr_number(handoff.pr_number)
    and core._is_bounded_string(handoff.version, core._max_dedup_len)
    and core.is_safe_pr_review_result_ref(handoff.review_proposal_id, handoff.review_dedup_key)
    and core.is_safe_head_sha(handoff.reviewed_head_sha)
    and core.is_safe_head_sha(handoff.current_head_sha)
    and source_refs.has_bounded_source_ref(handoff.source_ref, core._max_key_len) then
    return handoff
  end
  if handoff.kind == "github-devloop.fixing"
    and core.is_safe_entity_proposal_ref(handoff.proposal_id, handoff.version)
    and core.is_safe_pr_number(handoff.pr_number)
    and core._is_bounded_string(handoff.version, core._max_dedup_len)
    and core.is_safe_pr_review_result_ref(handoff.review_proposal_id, handoff.review_dedup_key)
    and core.is_safe_head_sha(handoff.reviewed_head_sha)
    and (handoff.current_head_sha == nil or core.is_safe_head_sha(handoff.current_head_sha))
    and (handoff.blocking_gap == nil or core._is_bounded_string(handoff.blocking_gap, core._max_blocking_gap_len))
    and (handoff.framing == nil or core._is_bounded_string(handoff.framing, core._max_framing_len))
    and (handoff.gate_baseline_sha == nil or core.is_safe_head_sha(handoff.gate_baseline_sha))
    and (handoff.gate_failure_excerpt == nil or core._is_bounded_string(handoff.gate_failure_excerpt, core._max_rollup_failure_summary_len))
    and (handoff.predecessor_set == nil or core._is_path_safe_key(handoff.predecessor_set, core._max_dedup_len))
    and (handoff.dedup_key == nil or core._is_path_safe_key(handoff.dedup_key, core._max_dedup_len))
    and source_refs.has_bounded_source_ref(handoff.source_ref, core._max_key_len) then
    return handoff
  end
  return nil
end

local function accept_handoff(event)
  return supported_handoff((event and event.payload) or {}) ~= nil
end

local function handoff_done(_event)
  return false
end

local function log_unsupported_handoff(event)
  local payload = event.payload or {}
  local handoff = supported_handoff(payload)
  if handoff == nil then
    local proposal_id = type(payload.handoff) == "table" and tostring(payload.handoff.proposal_id or "unknown") or "unknown"
    core.log_entry("comment_handoff", event, proposal_id, core.payload_field(payload, "dedup_key"))
    core.log_cas_decision("comment_handoff", proposal_id, { state = nil, version = nil }, "comment-written", "handoff", "skip-foreign(payload)", "unsupported comment-written handoff")
    return
  end
end

local function act_handoff(event)
  local payload = event.payload or {}
  local handoff = supported_handoff(payload)
  if handoff == nil then
    log_unsupported_handoff(event)
    return
  end

  core.log_entry("comment_handoff", event, handoff.proposal_id, payload.dedup_key)
  if handoff.kind == "github-devloop.ready" then
    local ready = core.build_devloop_ready_payload({
      proposal_id = handoff.proposal_id,
      dedup_key = handoff.marker_version,
      source_ref = handoff.source_ref,
      include_ready_hand_off = true,
      ready_comment_id = payload.comment_id,
    })
    core.log_cas_decision("comment_handoff", handoff.proposal_id, { state = "ready", version = ready.dedup_key }, "comment-written", "devloop_ready", "applied(own-write-comment-id)", "ready marker comment write was acknowledged")
    core.log_raise("comment_handoff", handoff.proposal_id, "devloop_ready", ready)
    return
  end

  if handoff.kind == "github-devloop.reconcile" then
    local reconcile = core.build_devloop_reconcile_payload({
      proposal_id = handoff.proposal_id,
      source_ref = handoff.source_ref,
    }, handoff.round, handoff.base_version)
    core.log_cas_decision("comment_handoff", handoff.proposal_id, { state = "thinking", version = handoff.base_version }, "comment-written", "devloop_reconcile", "applied(own-write-comment-id)", "converge round comment write was acknowledged")
    core.log_raise("comment_handoff", handoff.proposal_id, "devloop_reconcile", reconcile)
    return
  end

  if handoff.kind == "github-devloop.merge_ready" then
    local merge_ready = core.build_devloop_merge_ready_payload(handoff.proposal_id, handoff.pr_number, handoff.version, {
      review_proposal_id = handoff.review_proposal_id,
      review_dedup_key = handoff.review_dedup_key,
      reviewed_head_sha = handoff.reviewed_head_sha,
      current_head_sha = handoff.current_head_sha,
    }, handoff.source_ref)
    core.log_cas_decision("comment_handoff", handoff.proposal_id, { state = "merge-ready", version = handoff.version }, "comment-written", "devloop_merge_ready", "applied(own-write-comment-id)", "merge-ready marker comment write was acknowledged")
    core.log_raise("comment_handoff", handoff.proposal_id, "devloop_merge_ready", merge_ready)
    return
  end

  local entity = core.parse_entity_proposal_id(handoff.proposal_id)
  if handoff.kind == "github-devloop.fixing" then
    local fixing = core.build_devloop_fixing_payload({
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
    return
  end

  local claim_ok = false
  if entity == nil then
    claim_ok = false
  elseif entity.kind == "pr" then
    local repo = payload.repo
    if repo == nil then
      repo = select(1, core.parse_pr_source_ref(handoff.source_ref))
    end
    claim_ok = entity.repo == repo and tostring(entity.pr_number) == tostring(handoff.pr_number)
  else
    claim_ok = core.verify_pr_review_issue_claim("comment_handoff", entity.repo, entity.issue_number, nil, handoff.proposal_id)
  end
  if not claim_ok then
    return
  end
  local reviewing = core.build_devloop_reviewing_payload({
    proposal_id = handoff.proposal_id,
    impl_version = handoff.version,
    reviewing_comment_id = payload.comment_id,
  }, handoff.pr_number, handoff.source_ref, handoff.version)
  core.log_cas_decision("comment_handoff", handoff.proposal_id, { state = "reviewing", version = handoff.version }, "comment-written", "devloop_reviewing", "applied(own-write-comment-id)", "reviewing marker comment write was acknowledged")
  core.log_raise("comment_handoff", handoff.proposal_id, "devloop_reviewing", reviewing)
end

return saga.department(spec, {
  accept = accept_handoff,
  done = handoff_done,
  act = act_handoff,
  on_skip_foreign = log_unsupported_handoff,
  wrap = core.wrap_pipeline_failure,
  name = "comment_handoff",
})
