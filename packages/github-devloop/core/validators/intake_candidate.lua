return function(M)
function M.is_supported_intake_candidate(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-devloop.intake-candidate.v1"
    or not M.is_safe_proposal_ref(payload.proposal_id, payload.dedup_key)
    or not M._has_bounded_source_ref(payload.source_ref) then
    return false
  end
  local repo, issue_number = M.parse_issue_source_ref(payload.source_ref)
  return repo ~= nil
    and issue_number ~= nil
    and tostring(repo) == tostring(payload.repo)
    and tostring(issue_number) == tostring(payload.issue_number)
    and tostring(payload.proposal_id) == M.proposal_id(repo, issue_number)
end
end
