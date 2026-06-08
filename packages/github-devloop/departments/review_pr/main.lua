local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_reviewing" },
  produces = {
    "consensus.proposal",
  },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

function pipeline(event)
  local reviewing = event.payload or {}
  if not core.is_supported_reviewing(reviewing) then
    core.log_entry("review_pr", event, "unknown", reviewing.dedup_key)
    core.log_cas_decision("review_pr", "unknown", { state = nil, version = nil }, "reviewing", "review-proposal", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("review_pr", event, reviewing.proposal_id, reviewing.dedup_key)
  local repo, issue_number = core.parse_proposal_id(reviewing.proposal_id)
  if repo == nil then
    core.log_cas_decision("review_pr", reviewing.proposal_id, { state = nil, version = nil }, "reviewing", "review-proposal", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end

  local lock_key = core.review_lock_key(reviewing.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("review_pr", reviewing.proposal_id, { state = nil, version = nil }, "reviewing", "review-proposal", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local issue_view = exec_sync({ cmd = core.gh_issue_view_review_cmd(repo, issue_number), timeout = 30 })
    if issue_view.exit_code ~= 0 then
      error("github-devloop: gh issue review view failed: " .. tostring(issue_view.stderr))
    end

    local current_issue = core.parse_issue_view_review(issue_view.stdout)
    core.log_forged_markers("review_pr", reviewing.proposal_id, current_issue.comments)
    local state = core.current_state(current_issue.comments, reviewing.proposal_id)
    local marker_order = core.compare_state_marker_order(state, "reviewing", reviewing.version)
    if marker_order < 0 then
      core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "retry-pending(reviewing marker not yet visible)", "reviewing state marker not yet visible")
      error("github-devloop: reviewing state marker not yet visible for PR review; retrying")
    end
    if marker_order > 0 or state.state ~= "reviewing" then
      core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "skip-stale/diverged", "issue is not currently reviewing")
      return
    end

    if tostring(state.version or "") ~= tostring(reviewing.version) then
      core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "skip-stale(version-mismatch)", "reviewing event version does not match canonical issue marker")
      return
    end

    local pr_view = exec_sync({ cmd = core.gh_pr_view_origin_cmd(repo, reviewing.pr_number), timeout = 30 })
    if pr_view.exit_code ~= 0 then
      error("github-devloop: gh pr review head view failed: " .. tostring(pr_view.stderr))
    end
    local current_pr = core.parse_pr_view_origin(pr_view.stdout)
    if not core.is_safe_head_sha(current_pr.head_sha) then
      error("github-devloop: gh pr review head view returned unsafe head sha")
    end
    if tostring(current_pr.state or ""):lower() ~= "open" then
      core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "skip-stale(pr-closed)", "re-derived PR is not open")
      return
    end

    local diff = exec_sync({ cmd = core.gh_pr_diff_cmd(repo, reviewing.pr_number), timeout = 30 })
    if diff.exit_code ~= 0 then
      error("github-devloop: gh pr diff failed: " .. tostring(diff.stderr))
    end

    local after_diff_pr_view = exec_sync({ cmd = core.gh_pr_view_origin_cmd(repo, reviewing.pr_number), timeout = 30 })
    if after_diff_pr_view.exit_code ~= 0 then
      error("github-devloop: gh pr review head recheck failed: " .. tostring(after_diff_pr_view.stderr))
    end
    local after_diff_pr = core.parse_pr_view_origin(after_diff_pr_view.stdout)
    if tostring(after_diff_pr.head_ref_name or "") ~= tostring(current_pr.head_ref_name or "")
      or tostring(after_diff_pr.head_sha or "") ~= tostring(current_pr.head_sha or "") then
      error("github-devloop: PR head moved while reading review diff; retrying")
    end
    if tostring(after_diff_pr.state or ""):lower() ~= "open" then
      core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "skip-stale(pr-closed)", "re-derived PR is not open after diff")
      return
    end

    local pr_source_ref = {
      kind = "external",
      ref = tostring(repo) .. "#pr/" .. tostring(reviewing.pr_number),
    }
    local proposal = core.build_pr_review_proposal(repo, issue_number, reviewing.pr_number, reviewing.version, current_pr.head_sha, current_issue, diff.stdout, pr_source_ref)
    if not core.validate_proposal(proposal) then
      log.warn("github-devloop dept=review_pr proposal_id=" .. tostring(reviewing.proposal_id) .. " tag=SKIP reason=cannot-build-valid-review-proposal")
      return
    end

    core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "applied", "raising PR diff review proposal")
    core.log_apply("review_pr", reviewing.proposal_id, nil, nil, { add = {}, remove = {} }, {
      "consensus.proposal",
    })
    core.log_raise("review_pr", reviewing.proposal_id, "consensus.proposal", proposal)
  end)
end

return M
