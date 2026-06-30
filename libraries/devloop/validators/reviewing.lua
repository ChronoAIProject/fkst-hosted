local source_refs = require("contract.source_ref")

local payloads_predicates = require("devloop.payloads.predicates")
return function(M)
function M.is_supported_reviewing(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.reviewing.v1"
    and M.is_safe_entity_proposal_ref(payload.proposal_id, payload.dedup_key)
    and M.is_safe_pr_number(payload.pr_number)
    and M._is_bounded_string(payload.version, M._max_dedup_len)
    and (payload.reviewing_hand_off == nil
      or payloads_predicates.is_own_state_marker_hand_off(M, payload.reviewing_hand_off, {
        proposal_id = payload.proposal_id,
        state = "reviewing",
        marker_version = payload.version,
        event_version = payload.version,
      }))
    and source_refs.has_bounded_source_ref(payload.source_ref, M._max_key_len)
end
end
