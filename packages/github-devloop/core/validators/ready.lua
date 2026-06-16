local source_refs = require("std.source_ref")

return function(M)
function M.is_supported_ready(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.ready.v1"
    and M.is_safe_proposal_ref(payload.proposal_id, payload.dedup_key)
    and (payload.framing == nil or M._is_bounded_string(payload.framing, M._max_framing_len))
    and (payload.ready_hand_off == nil
      or (payload.impl_retry_attempt == nil
        and M.is_own_state_marker_hand_off(payload.ready_hand_off, {
          proposal_id = payload.proposal_id,
          state = "ready",
          marker_version = payload.ready_hand_off.marker_version,
          event_version = payload.dedup_key,
          effects = "result-marker,ready-label,devloop-ready",
        })))
    and (payload.impl_retry_attempt == nil
      or (tonumber(payload.impl_retry_attempt) ~= nil
        and tonumber(payload.impl_retry_attempt) >= 1
        and tonumber(payload.impl_retry_attempt) == math.floor(tonumber(payload.impl_retry_attempt))
        and tonumber(payload.impl_retry_attempt) <= M._max_impl_retry_attempts))
    and source_refs.has_bounded_source_ref(payload.source_ref, M._max_key_len)
end
end
