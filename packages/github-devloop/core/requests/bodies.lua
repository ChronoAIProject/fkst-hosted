local S = {}

function S.install(M, shared)
function M.build_merging_comment_body(merge_ready)
  return M.comment_string("is_merging_pr_prefix") .. tostring(merge_ready.pr_number)
    .. "\n\n" .. M.state_marker(merge_ready.proposal_id, "merging", merge_ready.version)
    .. "\n" .. M.merging_marker(merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha)
end

function M.build_merged_comment_body(merge_ready, autonomy_record)
  local autonomy_marker = ""
  if autonomy_record ~= nil then
    autonomy_marker = "\n" .. M.autonomy_result_marker(autonomy_record)
  end
  return M.comment_string("merged_pr_prefix") .. tostring(merge_ready.pr_number)
    .. "\n\n" .. M.state_marker(merge_ready.proposal_id, "merging", merge_ready.version)
    .. "\n" .. M.merging_marker(merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha)
    .. "\n" .. M.state_marker(merge_ready.proposal_id, "merged", merge_ready.version)
    .. "\n" .. M.merged_marker(merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha, autonomy_record)
    .. autonomy_marker
end
end

return S
