return function(M)
function M.is_supported_merge_ready(payload)
  return type(payload) == "table"
    and payload.schema == "github-devloop.merge-ready.v1"
    and M.is_safe_entity_proposal_ref(payload.proposal_id, payload.dedup_key)
    and M.is_safe_pr_number(payload.pr_number)
    and M._is_bounded_string(payload.version, M._max_dedup_len)
    and M.is_safe_pr_review_result_ref(payload.review_proposal_id, payload.review_dedup_key)
    and M._is_git_sha(payload.reviewed_head_sha)
    and M._has_bounded_source_ref(payload.source_ref)
end
end
