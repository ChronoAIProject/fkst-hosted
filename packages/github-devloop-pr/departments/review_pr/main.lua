local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local base_ids = require("devloop.base_ids")
local m_claims = require("devloop.claims")
local parsers_pr = require("devloop.parsers.pr")
local parsers_issue = require("devloop.parsers.issue")
local core, saga, context_bundle = require("core"), require("workflow.saga"), require("devloop.context_bundle")
local transition_version = require("contract.transition_version")

local payloads_builders = require("devloop.payloads.builders")
local payloads_predicates = require("devloop.payloads.predicates")
local v_reviewing = require("devloop.validators.reviewing")
local v_validate_proposal = require("devloop.validators.validate_proposal")
-- Preserve existing body line coordinates for the coverage ratchet.

local spec = {
  consumes = { "devloop_reviewing" },
  produces = {
    "consensus.proposal",
  },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local function reviewing_transition_status(state, reviewing_version)
  if state == nil or state.version == nil then
    return "pending"
  end

  local state_base = transition_version.strip_suffixes(state.version)
  local reviewing_base = transition_version.strip_suffixes(reviewing_version)
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

return saga.department(spec, { done = function() return false end, act = function(event)
  local reviewing = event.payload or {}
  if not v_reviewing.is_supported_reviewing(core, reviewing) then
    core.log_entry("review_pr", event, "unknown", core.payload_field(reviewing, "dedup_key"))
    core.log_cas_decision("review_pr", "unknown", { state = nil, version = nil }, "reviewing", "review-proposal", "skip-foreign(payload)", "unsupported event payload")
    return
  end

  core.log_entry("review_pr", event, reviewing.proposal_id, reviewing.dedup_key)
  local entity = entity_lib.parse_entity_proposal_id(reviewing.proposal_id)
  if entity == nil then
    core.log_cas_decision("review_pr", reviewing.proposal_id, { state = nil, version = nil }, "reviewing", "review-proposal", "skip-foreign(proposal_id)", "proposal_id is outside github-devloop")
    return
  end
  local repo = entity.repo
  local issue_number = entity.issue_number

  local lock_key = entity_lib.review_lock_key(reviewing.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("review_pr", reviewing.proposal_id, { state = nil, version = nil }, "reviewing", "review-proposal", "skip-foreign(proposal_id)", "no transition lock key")
    return
  end

  with_lock(lock_key, function()
    devloop_base.assert_trusted_bot_configured()

    local pr_view = core.gh_pr_view_origin(repo, reviewing.pr_number, 30)
    if pr_view.exit_code ~= 0 then
      error("github-devloop: gh pr review head view failed: " .. tostring(pr_view.stderr))
    end
    local current_pr = parsers_pr.parse_pr_view_origin(core, pr_view.stdout)
    core.log_forged_markers("review_pr", reviewing.proposal_id, current_pr.comments)
    local state = require("devloop.entity").current_entity_state(core, current_pr.comments, reviewing.proposal_id)
    local transition = reviewing_transition_status(state, reviewing.version)
    if transition == "pending" or transition == "version-mismatch" then
      local verified_state = nil
      local hand_off_reason = "missing"
      if reviewing.reviewing_hand_off ~= nil then
        verified_state, hand_off_reason = payloads_predicates.verified_hand_off_state(core, repo, reviewing.reviewing_hand_off, {
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

    if not require("devloop.pr_safety").is_safe_head_sha(current_pr.head_sha) then
      error("github-devloop: gh pr review head view returned unsafe head sha")
    end
    if tostring(current_pr.state or ""):lower() ~= "open" then
      core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "skip-stale(pr-closed)", "re-derived PR is not open")
      return
    end

    local pr_source_ref = entity_lib.pr_source_ref(repo, reviewing.pr_number)
    local current_issue = {
      title = "PR #" .. tostring(reviewing.pr_number),
      body = "(PR-only review context; issue backing is absent)",
      comments = current_pr.comments,
    }
    if issue_number ~= nil then
      local issue_view = core.gh_issue_view_review(repo, issue_number, 30)
      if issue_view.exit_code ~= 0 then
        error("github-devloop: gh issue review view failed: " .. tostring(issue_view.stderr))
      end
      current_issue = parsers_issue.parse_issue_view_review(core, issue_view.stdout)
    end
    if not m_claims.verify_pr_review_issue_claim(core, "review_pr", repo, issue_number, current_issue, reviewing.proposal_id) then
      return
    end
    local review_id = devloop_base.pr_review_proposal_id(repo, reviewing.pr_number, reviewing.version, current_pr.head_sha)
    local review_dedup_key = base_ids.dedup_key({ review_id, "review" })
    local context_fetch = { context_bundle.context_fetch_ref_from_bundle(core, {
      dept = "review_pr",
      repo = repo,
      issue_number = issue_number,
      pr_number = reviewing.pr_number,
      proposal_id = review_id,
      version = review_dedup_key,
      tick = event.ts,
    }) }
    local content_fetch = context_fetch[1]
    local high_risk = context_fetch[2]
    local proposal = payloads_builders.build_board_pr_review_proposal(core, repo, issue_number, reviewing.pr_number, reviewing.version, current_pr.head_sha, current_issue, pr_source_ref, event.ts, current_pr.comments, content_fetch, high_risk)
    if not v_validate_proposal.validate_proposal(core, proposal) then
      log.warn("github-devloop dept=review_pr proposal_id=" .. tostring(reviewing.proposal_id) .. " tag=SKIP reason=cannot-build-valid-review-proposal")
      return
    end

    core.log_cas_decision("review_pr", reviewing.proposal_id, state, "reviewing", "review-proposal", "applied", "raising PR diff review proposal")
    local raised = { "consensus.proposal" }
    core.log_apply("review_pr", reviewing.proposal_id, nil, nil, { add = {}, remove = {} }, raised)
    core.log_raise("review_pr", reviewing.proposal_id, "consensus.proposal", proposal)
  end)
end, wrap = core.wrap_pipeline_failure, name = "review_pr" })
