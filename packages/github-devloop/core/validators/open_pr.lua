local source_refs = require("std.source_ref")

return function(M)
function M.is_supported_open_pr(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "github-devloop.open-pr.v1"
    or not M.is_safe_entity_proposal_ref(payload.proposal_id, payload.dedup_key)
    or not M._is_bounded_string(payload.version, M._max_dedup_len)
    or not M._is_git_ref_safe(payload.branch)
    or not M._is_git_sha(payload.head_sha)
    or not M._is_git_ref_safe(payload.base_branch)
    or not source_refs.has_bounded_source_ref(payload.source_ref, M._max_key_len) then
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
