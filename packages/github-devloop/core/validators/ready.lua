return function(M)
function M.is_supported_ready(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.ready.v1"
    and M.is_safe_proposal_ref(payload.proposal_id, payload.dedup_key)
    and (payload.framing == nil or M._is_bounded_string(payload.framing, M._max_framing_len))
    and (payload.ready_hand_off == nil
      or (payload.impl_retry_attempt == nil and M.is_ready_hand_off(payload.ready_hand_off, payload)))
    and (payload.impl_retry_attempt == nil
      or (tonumber(payload.impl_retry_attempt) ~= nil
        and tonumber(payload.impl_retry_attempt) >= 1
        and tonumber(payload.impl_retry_attempt) == math.floor(tonumber(payload.impl_retry_attempt))
        and tonumber(payload.impl_retry_attempt) <= M._max_impl_retry_attempts))
    and M._has_bounded_source_ref(payload.source_ref)
end
end
