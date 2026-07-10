local entity_lib = require("devloop.entity")
local M = {}
local check_runs = require("forge.github.check_runs")
local git_mechanics = require("devloop.git_mechanics")
local m_mgw = require("devloop.merge_gate_wait")
local devloop_logging = require("devloop.logging")
local pr_safety = require("devloop.pr_safety")

function M.should_wait_for_stale_mergeability(core, pr, branches, mergeable_reason)
  if not check_runs.is_not_mergeable_reason(mergeable_reason) then
    return false, "not-stale-mergeability"
  end
  local base_head, base_reason = git_mechanics.current_base_head(core.git, branches.integration)
  if base_head == nil then
    return false, base_reason
  end
  local head_sha = tostring(pr and pr.head_sha or "")
  if not pr_safety.is_safe_head_sha(head_sha) then
    return false, "unsafe-pr-head"
  end
  local result = git_mechanics.git_is_ancestor(core.git, base_head, head_sha, 30)
  if result.exit_code == 0 then
    return true, "current-base-contained"
  end
  return false, "current-base-not-contained"
end

function M.hold(core, merge_ready, repo, current_pr, classification)
  local reason = tostring(classification and classification.reason or "ci-wait")
  local source_ref = entity_lib.pr_source_ref(repo, merge_ready.pr_number)
  local comment_request = m_mgw.build_merge_gate_wait_comment_request(repo,
    merge_ready,
    reason,
    classification and classification.kind or "CI_WAIT",
    source_ref
  )
  devloop_logging.log_raise("merge", merge_ready.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  devloop_logging.log_line("info", "merge", merge_ready.proposal_id, "GATE", {
    "pr=" .. tostring(merge_ready.pr_number),
    "version=" .. tostring(merge_ready.version),
    "outcome=hold",
    "reason=" .. reason,
    "ci_class=" .. tostring(classification and classification.kind or ""),
    "head_sha=" .. tostring(current_pr and current_pr.head_sha or ""),
  })
  error("github-devloop: merge-ci-wait: merge wait on " .. reason .. "; retrying")
end

return M
