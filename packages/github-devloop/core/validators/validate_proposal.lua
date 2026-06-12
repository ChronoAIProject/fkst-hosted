return function(M)
function M.validate_proposal(proposal)
  if type(proposal) ~= "table" then
    return false
  end
  if proposal.schema ~= "consensus.proposal.v1" then
    return false
  end
  local repo, issue_number = M.parse_proposal_id(proposal.proposal_id)
  if repo == nil or issue_number == nil then
    local review_repo, pr_number = M.parse_pr_review_proposal_id(proposal.proposal_id)
    if review_repo == nil or pr_number == nil then
      return false
    end
    if not M._is_path_safe_key(proposal.proposal_id, M._max_key_len) or not M._is_path_safe_key(proposal.dedup_key, M._max_dedup_len) then
      return false
    end
  else
    if not M.is_safe_proposal_ref(proposal.proposal_id, proposal.dedup_key) then
      return false
    end
  end
  if not M._is_bounded_string(proposal.title, M._max_title_len) then
    return false
  end
  if not M._is_bounded_string(proposal.body, M._max_body_len) then
    return false
  end
  if proposal.content_fetch ~= nil and not M._is_bounded_string(proposal.content_fetch, 4000) then
    return false
  end
  return M._has_bounded_source_ref(proposal.source_ref)
end
end
