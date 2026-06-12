return function(M)
function M.is_supported_result(payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_reached.v1"
    and payload.decision == "approve"
    and M.is_safe_consensus_result_ref(payload.proposal_id, payload.dedup_key)
    and M._is_bounded_string(payload.body, M._max_body_len)
    and (payload.framing == nil or M._is_bounded_string(payload.framing, M._max_framing_len))
    and M._has_bounded_source_ref(payload.source_ref)
end
end
