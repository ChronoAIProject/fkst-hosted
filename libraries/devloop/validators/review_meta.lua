local source_refs = require("contract.source_ref")

return function(M)
function M.is_supported_review_meta(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-devloop.review-meta.v1"
    or not M.is_safe_pr_review_result_ref(payload.review_proposal_id, payload.review_dedup_key) then
    return false
  end
  local has_valid_identity = payload.mode == "fix-reflection"
    and M.parse_entity_proposal_id(payload.proposal_id) ~= nil
    and M._is_path_safe_key(payload.dedup_key, M._max_dedup_len)
  if payload.mode ~= "fix-reflection" then
    has_valid_identity = M.is_safe_entity_proposal_ref(payload.proposal_id, payload.dedup_key)
  end
  return has_valid_identity
    and M._is_bounded_string(payload.version, M._max_dedup_len)
    and M.is_safe_pr_number(payload.pr_number)
    and tonumber(payload.n) ~= nil
    and (payload.mode == nil or payload.mode == "fix-reflection")
    and (payload.fix_round == nil or tonumber(payload.fix_round) ~= nil)
    and (payload.blocking_gap == nil or M._is_bounded_string(payload.blocking_gap, M._max_blocking_gap_len))
    and source_refs.has_bounded_source_ref(payload.source_ref, M._max_key_len)
end
end
