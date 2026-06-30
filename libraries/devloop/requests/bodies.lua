local S = {}
local autonomy_ledger = require("devloop.autonomy_ledger")
local comment_strings = require("devloop.strings")

function S.install(M, shared)
function M.build_merging_comment_body(merge_ready)
  return comment_strings.comment_string(M, "is_merging_pr_prefix") .. tostring(merge_ready.pr_number)
    .. "\n\n" .. M.state_marker(merge_ready.proposal_id, "merging", merge_ready.version)
    .. "\n" .. M.merging_marker(merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha)
end

function M.build_merged_comment_body(merge_ready, autonomy_record)
  local autonomy_marker = ""
  if autonomy_record ~= nil then
    autonomy_marker = "\n" .. autonomy_ledger.autonomy_result_marker(M, autonomy_record)
  end
  return comment_strings.comment_string(M, "merged_pr_prefix") .. tostring(merge_ready.pr_number)
    .. "\n\n" .. M.state_marker(merge_ready.proposal_id, "merging", merge_ready.version)
    .. "\n" .. M.merging_marker(merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha)
    .. "\n" .. M.state_marker(merge_ready.proposal_id, "merged", merge_ready.version)
    .. "\n" .. M.merged_marker(merge_ready.proposal_id, merge_ready.pr_number, merge_ready.version, merge_ready.reviewed_head_sha, autonomy_record)
    .. autonomy_marker
end
end

return S
