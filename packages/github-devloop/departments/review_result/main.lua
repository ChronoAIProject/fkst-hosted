local core = require("core")

local M = {}

M.spec = {
  consumes = { "consensus.consensus_reached" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
    "devloop_fixing",
  },
  fanout = { "consensus.consensus_reached" },
  stall_window = "30s",
}

function pipeline(event)
  local reached = event.payload or {}
  if not core.is_supported_review_result(reached) then
    core.log_entry("review_result", event, "unknown", reached.dedup_key)
    core.log_cas_decision("review_result", "unknown", { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("review_result", event, reached.proposal_id, reached.dedup_key)
  local review_repo, proposal_pr_number, review_version, reviewed_head_sha = core.parse_pr_review_proposal_id(reached.proposal_id)
  if review_repo == nil then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop pr-review")
    return
  end
  local repo, pr_number = core.parse_pr_source_ref(reached.source_ref)
  if repo == nil or tostring(pr_number) ~= tostring(proposal_pr_number) then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(source_ref)", "review source_ref does not match PR review proposal")
    return
  end

  core.assert_trusted_bot_configured()
  local pr_view = exec_sync({ cmd = core.gh_pr_view_origin_cmd(repo, pr_number), timeout = 30 })
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr origin view failed for review result: " .. tostring(pr_view.stderr))
  end
  local current_pr = core.parse_pr_view_origin(pr_view.stdout)
  local origin = core.pr_origin_fact(current_pr.comments)
  if origin == nil then
    if core.is_devloop_issue_branch(current_pr.head_ref_name) then
      core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "retry-pending(pr-origin)", "trusted PR origin marker not yet visible")
      error("github-devloop: trusted pr-origin marker not yet visible for review result; retrying")
    end
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(pr-origin)", "trusted PR origin marker absent")
    return
  end
  if origin.repo ~= repo then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(repo)", "pr-origin repo mismatch")
    return
  end
  if tostring(current_pr.head_ref_name or "") ~= tostring(origin.branch) then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(head)", "pr-origin branch mismatch")
    return
  end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-stale(pr-closed)", "re-derived PR is not open")
    return
  end
  if tostring(current_pr.head_sha or "") ~= tostring(reviewed_head_sha) then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-stale(head-advanced)", "PR head advanced since reviewed diff")
    return
  end
  local reviewed_issue_version = tostring(review_version or "")
  if reviewed_issue_version == "" then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(version)", "review proposal version is missing")
    return
  end

  local lock_key = core.review_result_lock_key(origin.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("review_result", reached.proposal_id, { state = nil, version = nil }, "reviewing", "merge-ready|fixing", "skip-foreign(proposal_id)", "no issue transition lock key")
    return
  end

  with_lock(lock_key, function()
    local issue_source_ref = {
      kind = "external",
      ref = tostring(origin.repo) .. "#issue/" .. tostring(origin.issue_number),
    }
    local issue_view = exec_sync({ cmd = core.gh_issue_view_result_cmd(origin.repo, origin.issue_number), timeout = 30 })
    if issue_view.exit_code ~= 0 then
      error("github-devloop: gh issue review result view failed: " .. tostring(issue_view.stderr))
    end

    local current_issue = core.parse_issue_view_result(issue_view.stdout)
    core.log_forged_markers("review_result", origin.proposal_id, current_issue.comments)
    local state = core.current_state(current_issue.comments, origin.proposal_id)
    local to_state = reached.decision == "approve" and "merge-ready" or "fixing"
    local current_review_version = core.safe_version_segment(state.version or "")
    local transition = core.cyclic_transition_status({
      state = state.state,
      version = current_review_version,
      stage_rank = state.stage_rank,
    }, { "reviewing" }, to_state, reviewed_issue_version)
    if transition == "idempotent" or transition == "stale" then
      core.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, core.cas_outcome(state, transition, reached.dedup_key), "review decision cannot advance current marker")
      return
    end
    if transition == "pending" then
      core.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, core.cas_outcome(state, transition, reached.dedup_key), "reviewing state marker not yet visible")
      error("github-devloop: reviewing marker not yet visible for review result; retrying")
    end

    if tostring(current_review_version) ~= tostring(reviewed_issue_version) then
      core.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, "skip-stale(version-mismatch)", "PR origin implementation version does not match canonical issue marker")
      return
    end

    core.log_cas_decision("review_result", origin.proposal_id, state, "reviewing", to_state, core.cas_outcome(state, transition, reached.dedup_key), "review decision=" .. tostring(reached.decision))
    local issue_version = state.version
    if reached.decision == "reject" then
      issue_version = core.fix_version_from_review_version(state.version)
    end
    local comment_request = core.build_review_result_comment_request(origin.repo, origin.issue_number, origin.proposal_id, issue_version, reached, issue_source_ref)
    local label_request = core.build_review_result_label_request(origin.repo, origin.issue_number, origin.proposal_id, reached, issue_source_ref)
    local add_labels, remove_labels = core.state_label_changes(to_state)
    local raised = {
      "github-proxy.github_issue_comment_request",
      "github-proxy.github_issue_label_request",
    }
    local fix_payload = nil
    if reached.decision == "reject" then
      fix_payload = core.build_devloop_fixing_payload(origin, pr_number, {
        review_proposal_id = reached.proposal_id,
        review_dedup_key = reached.dedup_key,
        reviewed_head_sha = reviewed_head_sha,
        fix_version = issue_version,
      }, issue_source_ref)
      table.insert(raised, "devloop_fixing")
    end
    core.log_apply("review_result", origin.proposal_id, to_state, issue_version, { add = add_labels, remove = remove_labels }, raised)
    core.log_raise("review_result", origin.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
    core.log_raise("review_result", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
    if fix_payload ~= nil then
      core.log_raise("review_result", origin.proposal_id, "devloop_fixing", fix_payload)
    end
  end)
end

return M
