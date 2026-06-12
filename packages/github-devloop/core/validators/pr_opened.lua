return function(M)
function M.is_supported_pr_opened(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-proxy.pr-opened.v1"
    or not M.is_safe_entity_proposal_ref(payload.proposal_id, payload.dedup_key)
    or not M.is_safe_pr_number(payload.pr_number)
    or not M._is_bounded_string(payload.impl_version, M._max_dedup_len)
    or not M._is_git_ref_safe(payload.branch)
    or not M._is_git_sha(payload.head_sha)
    or not M._is_git_ref_safe(payload.base_branch)
    or not M._has_bounded_source_ref(payload.source_ref) then
    return false
  end
  local source_repo, source_pr = M.parse_pr_source_ref(payload.source_ref)
  local issue_repo, issue_number = M.parse_proposal_id(payload.proposal_id)
  return source_repo ~= nil
    and source_pr ~= nil
    and issue_repo ~= nil
    and issue_number ~= nil
    and tostring(source_repo) == tostring(payload.repo)
    and tostring(source_pr) == tostring(payload.pr_number)
    and tostring(issue_repo) == tostring(payload.repo)
    and tostring(issue_number) == tostring(payload.issue_number)
end
end
