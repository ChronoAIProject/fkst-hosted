local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_reviewing" },
  produces = {
    "github-proxy.github_pr_comment_request",
    "consensus.proposal",
  },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

function pipeline(event)
  local reviewing = event.payload or {}
  if not core.is_supported_reviewing(reviewing) then
    core.log_entry("review_pr", event, "unknown", core.payload_field(reviewing, "dedup_key"))
    core.log_cas_decision("review_pr", "unknown", { state = nil, version = nil }, "reviewing", "review-proposal", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("review_pr", event, reviewing.proposal_id, reviewing.dedup_key)
  local entity = core.parse_entity_proposal_id(reviewing.proposal_id)
  if entity == nil then
    core.log_cas_decision("review_pr", reviewing.proposal_id, { state = nil, version = nil }, "reviewing", "review-proposal", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number

  local lock_key = core.review_lock_key(reviewing.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("review_pr", reviewing.proposal_id, { state = nil, version = nil }, "reviewing", "review-proposal", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    core.assert_trusted_bot_configured()

    local pr_view = core.gh_exec({ cmd = core.gh_pr_view_origin_cmd(repo, reviewing.pr_number), timeout = 30 })
    if pr_view.exit_code ~= 0 then
      error("github-devloop: gh pr review head view failed: " .. tostring(pr_view.stderr))
    end
    local current_pr = core.parse_pr_view_origin(pr_view.stdout)
    core.log_forged_markers("review_pr", reviewing.proposal_id, current_pr.comments)
    local state = core.current_entity_state(current_pr.comments, reviewing.proposal_id)
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

    if not core.is_safe_head_sha(current_pr.head_sha) then
      error("github-devloop: gh pr review head view returned unsafe head sha")
    end
    if tostring(current_pr.state or ""):lower() ~= "open" then
      core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "skip-stale(pr-closed)", "re-derived PR is not open")
      return
    end

    local pr_source_ref = core.pr_source_ref(repo, reviewing.pr_number)
    local current_issue = {
      title = "PR #" .. tostring(reviewing.pr_number),
      body = "(PR-only review context; issue backing is absent)",
      comments = current_pr.comments,
    }
    if issue_number ~= nil then
      local issue_view = core.gh_exec({ cmd = core.gh_issue_view_review_cmd(repo, issue_number), timeout = 30 })
      if issue_view.exit_code ~= 0 then
        error("github-devloop: gh issue review view failed: " .. tostring(issue_view.stderr))
      end
      current_issue = core.parse_issue_view_review(issue_view.stdout)
    end
    local review_id = core.pr_review_proposal_id(repo, reviewing.pr_number, reviewing.version, current_pr.head_sha)
    local content_fetch = core.context_fetch_ref_from_bundle({
      dept = "review_pr",
      repo = repo,
      issue_number = issue_number,
      pr_number = reviewing.pr_number,
      proposal_id = review_id,
      version = core._dedup_key({ review_id, "review" }),
      tick = event.ts,
    })
    local proposal = core.build_board_pr_review_proposal(repo, issue_number, reviewing.pr_number, reviewing.version, current_pr.head_sha, current_issue, pr_source_ref, event.ts, current_pr.comments, content_fetch)
    if not core.validate_proposal(proposal) then
      log.warn("github-devloop dept=review_pr proposal_id=" .. tostring(reviewing.proposal_id) .. " tag=SKIP reason=cannot-build-valid-review-proposal")
      return
    end

    core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "applied", "raising PR diff review proposal")
    local card_request = core.build_work_card_comment_request({
      kind = "pr",
      repo = repo,
      number = reviewing.pr_number,
    }, {
      proposal_id = reviewing.proposal_id,
      role = "review",
      version = reviewing.version,
      round = core.version_fix_round(reviewing.version),
      started_at = now(),
      source_ref = pr_source_ref,
    })
    local raised = { "consensus.proposal" }
    if core.write_mode() == "real" then
      table.insert(raised, 1, "github-proxy.github_pr_comment_request")
    end
    core.log_apply("review_pr", reviewing.proposal_id, nil, nil, { add = {}, remove = {} }, raised)
    core.log_work_card("review_pr", reviewing.proposal_id, "github-proxy.github_pr_comment_request", card_request)
    core.log_raise("review_pr", reviewing.proposal_id, "consensus.proposal", proposal)
  end)
end

return M
