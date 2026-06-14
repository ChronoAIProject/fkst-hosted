local core = require("core")

local M = {}

M.spec = {
  consumes = { "github-proxy.github_comment_written" },
  produces = {
    "devloop_ready",
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
    and core._has_bounded_source_ref(handoff.source_ref) then
    return handoff
  end
  if handoff.kind == "github-devloop.reviewing"
    and core.is_safe_entity_proposal_ref(handoff.proposal_id, handoff.version)
    and core.is_safe_pr_number(handoff.pr_number)
    and core._is_bounded_string(handoff.version, core._max_dedup_len)
    and core._has_bounded_source_ref(handoff.source_ref) then
    return handoff
  end
  return nil
end

function pipeline(event)
  local payload = event.payload or {}
  local handoff = supported_handoff(payload)
  if handoff == nil then
    local proposal_id = type(payload.handoff) == "table" and tostring(payload.handoff.proposal_id or "unknown") or "unknown"
    core.log_entry("comment_handoff", event, proposal_id, core.payload_field(payload, "dedup_key"))
    core.log_cas_decision("comment_handoff", proposal_id, { state = nil, version = nil }, "comment-written", "handoff", "skip-foreign(payload)", "unsupported comment-written handoff")
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

  local entity = core.parse_entity_proposal_id(handoff.proposal_id)
  if entity == nil or not core.verify_pr_review_issue_claim("comment_handoff", entity.repo, entity.issue_number, nil, handoff.proposal_id) then
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

pipeline = core.wrap_pipeline_failure("comment_handoff", pipeline)

return M
