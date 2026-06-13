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

local function reviewing_transition_status(state, reviewing_version)
  if state == nil or state.version == nil then
    return "pending"
  end

  local state_base = core.strip_transition_version_suffixes(state.version)
  local reviewing_base = core.strip_transition_version_suffixes(reviewing_version)
  if state.state == "reviewing" then
    if tostring(state_base) == tostring(reviewing_base) then
      return "apply"
    end
    return "version-mismatch"
  end

  local canonical_order = core.compare_state_marker_order({
    state = state.state,
    version = state_base,
  }, "reviewing", reviewing_base)
  if canonical_order < 0 then
    return "pending"
  end
  return "stale"
end

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
    local transition = reviewing_transition_status(state, reviewing.version)
    if transition == "pending" or transition == "version-mismatch" then
      local verified_state = nil
      local hand_off_reason = "missing"
      if reviewing.reviewing_hand_off ~= nil then
        verified_state, hand_off_reason = core.verified_hand_off_state(repo, reviewing.reviewing_hand_off, {
          proposal_id = reviewing.proposal_id,
          state = "reviewing",
          marker_version = reviewing.version,
          event_version = reviewing.version,
        })
      end
      if verified_state ~= nil then
        state = verified_state
        transition = "apply"
        core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "apply(verified-own-reviewing-hand-off)", "reviewing marker comment verified by direct id lookup")
      else
        if reviewing.reviewing_hand_off ~= nil then
          core.log_line("info", "review_pr", reviewing.proposal_id, "HANDOFF", {
            "state=reviewing",
            "outcome=verify-failed",
            "reason=" .. tostring(hand_off_reason),
          })
        end
        if transition == "version-mismatch" then
          core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "skip-stale(version-mismatch)", "reviewing event version does not match canonical issue marker")
          return
        end
        core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "retry-pending(reviewing marker not yet visible)", "reviewing state marker not yet visible")
        error("github-devloop: reviewing state marker not yet visible for PR review; retrying")
      end
    end
    if transition == "stale" then
      core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "skip-stale/diverged", "issue is not currently reviewing")
      return
    end

    if transition == "version-mismatch" then
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
    local review_dedup_key = core._dedup_key({ review_id, "review" })
    local content_fetch = core.context_fetch_ref_from_bundle({
      dept = "review_pr",
      repo = repo,
      issue_number = issue_number,
      pr_number = reviewing.pr_number,
      proposal_id = review_id,
      version = review_dedup_key,
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
      run_id = core.work_card_run_id({ "review", review_id, review_dedup_key }),
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

pipeline = core.wrap_pipeline_failure("review_pr", pipeline)

return M
