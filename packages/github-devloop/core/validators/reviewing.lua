return function(M)
function M.is_supported_reviewing(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.reviewing.v1"
    and M.is_safe_entity_proposal_ref(payload.proposal_id, payload.dedup_key)
    and M.is_safe_pr_number(payload.pr_number)
    and M._is_bounded_string(payload.version, M._max_dedup_len)
    and M._has_bounded_source_ref(payload.source_ref)
end
end
