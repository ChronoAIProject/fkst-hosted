local entity_lib = require("devloop.entity")
local M = {}
local m_mgw = require("devloop.merge_gate_wait")

function M.hold(core, merge_ready, repo, current_pr, classification)
  local reason = tostring(classification and classification.reason or "ci-wait")
  local source_ref = entity_lib.pr_source_ref(repo, merge_ready.pr_number)
  local comment_request = m_mgw.build_merge_gate_wait_comment_request(core,
    repo,
    merge_ready,
    reason,
    classification and classification.kind or "CI_WAIT",
    source_ref
  )
  core.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  core.log_line("info", "merge", merge_ready.proposal_id, "GATE", {
    "pr=" .. tostring(merge_ready.pr_number),
    "version=" .. tostring(merge_ready.version),
    "outcome=hold",
    "reason=" .. reason,
    "ci_class=" .. tostring(classification and classification.kind or ""),
    "head_sha=" .. tostring(current_pr and current_pr.head_sha or ""),
  })
  error("github-devloop: merge wait on " .. reason .. "; retrying")
end

return M
