local source_refs = require("std.source_ref")

return function(M)
function M.is_supported_review_result(payload)
  return type(payload) == "table"
    and payload.schema == "consensus.consensus_reached.v1"
    and (payload.decision == "approve" or payload.decision == "reject")
    and M.is_safe_pr_review_result_ref(payload.proposal_id, payload.dedup_key)
    and M._is_bounded_string(payload.body, M._max_body_len)
    and (payload.framing == nil or M._is_bounded_string(payload.framing, M._max_framing_len))
    and (payload.blocking_gap == nil or M._is_bounded_string(payload.blocking_gap, M._max_blocking_gap_len))
    and source_refs.has_bounded_source_ref(payload.source_ref, M._max_key_len)
end
end
