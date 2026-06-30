local source_refs = require("contract.source_ref")

local C = {}
function C.is_supported_result(M, payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_reached.v1"
    and payload.decision == "approve"
    and M.is_safe_consensus_result_ref(payload.proposal_id, payload.dedup_key)
    and (payload.effect_version == nil or M.is_safe_consensus_result_ref(payload.proposal_id, payload.effect_version))
    and M._is_bounded_string(payload.body, M._max_body_len)
    and (payload.framing == nil or M._is_bounded_string(payload.framing, M._max_framing_len))
    and source_refs.has_bounded_source_ref(payload.source_ref, M._max_key_len)
end

return C
