local entity_lib = require("devloop.entity")
local devloop_base = require("devloop.base")
local m_claims = require("devloop.claims")
local requests_labels = require("devloop.requests.labels")
local requests_review = require("devloop.requests.review")
local parsers_pr = require("devloop.parsers.pr")
local parsers_issue = require("devloop.parsers.issue")
local convergence_shared = require("devloop.convergence.shared")
local transition_version = require("contract.transition_version")
local core = require("core")
local context_bundle = require("devloop.context_bundle")
local config = require("devloop.config")

local saga = require("workflow.saga")

local payloads_builders = require("devloop.payloads.builders")
local conv_rounds = require("devloop.convergence.rounds")
local conv_reconcile = require("devloop.convergence.reconcile")
local v_pr_review_unresolved = require("devloop.validators.pr_review_unresolved")
local v_validate_proposal = require("devloop.validators.validate_proposal")
local m_facts = require("devloop.markers.facts")
local spec = {
  consumes = { "consensus.consensus_converge" },
  produces = {
    "consensus.proposal",
    "github-proxy.github_pr_comment_request",
    "github-proxy.github_issue_label_request",
    "devloop_review_meta",
    "devloop_review_reconcile",
  },
  fanout = { "consensus.consensus_converge" },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local function review_truth_table_unapproved(unresolved)
  if tonumber(unresolved.round) == nil or tonumber(unresolved.round) < 1 then
    return false
  end
  if type(unresolved.angle_digests) ~= "table" or #unresolved.angle_digests == 0 then
    return false
  end
  local has_comment = false
  for _, item in ipairs(unresolved.angle_digests) do
    local verdict = type(item) == "table" and item.verdict or nil
    if verdict == "approve" or verdict == "reject" or verdict == "invalid" then
      return false
    end
    if verdict == "comment" then
      has_comment = true
    elseif verdict == "abstain" then
    else
      return false
    end
  end
  if not has_comment then
    return true
  end
  return tostring(unresolved.dedup_key or ""):find("/loop/", 1, true) ~= nil
end

-- review_version is parse_pr_review_proposal_id's safe_version_segment form (truncated +
-- checksummed for long versions): it preserves version EQUALITY only, never ordering, so it
-- must NOT be fed to the ordering-based CAS. The PR head is already pinned to reviewed_head_sha
-- before the lock, so we only need: same reviewing version (segment equality) -> apply; issue
-- advanced past reviewing (stage_rank, which IS order-preserving) or reviewing at a different
-- version -> stale skip; not yet at reviewing -> pending retry.
local function reviewing_segment_transition_status(state, review_version)
  if state.state == "reviewing"
    and tostring(transition_version.safe_version_segment(state.version or "")) == tostring(review_version) then
    return "apply"
  end
  if state.state ~= nil and core.stage_rank(state.state) > core.stage_rank("reviewing") then
    return "stale"
  end
  if state.state == "reviewing" then
    -- reviewing but a different version segment (head already pinned): treat as version-mismatch stale, do not retry
    return "stale"
  end
  return "pending"  -- no marker yet, or a state earlier than reviewing -> reviewing marker not yet visible
end

return saga.department(spec, { done = function() return false end, act = function(event)
  local unresolved = event.payload or {}
  if not v_pr_review_unresolved.is_supported_pr_review_unresolved(core, unresolved) then
    core.log_entry("review_loop", event, "unknown", core.payload_field(unresolved, "dedup_key"))
    core.log_cas_decision("review_loop", "unknown", { state = nil, version = nil }, "reviewing", "reviewing|blocked", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("review_loop", event, unresolved.proposal_id, unresolved.dedup_key)
  local _, pr_number, review_version, reviewed_head_sha = devloop_base.parse_pr_review_proposal_id(unresolved.proposal_id)
  local repo, source_pr_number = devloop_base.parse_pr_source_ref(unresolved.source_ref)
  if repo == nil or tostring(source_pr_number) ~= tostring(pr_number) then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|blocked", "skip-foreign(source_ref)", "review source_ref does not match PR review proposal")
    return
  end

  devloop_base.assert_trusted_bot_configured()
  local branches = config.branch_config(core)
  local pr_view = core.gh_pr_view_origin(repo, pr_number, 30)
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr origin view failed for review loop: " .. tostring(pr_view.stderr))
  end
  local current_pr = parsers_pr.parse_pr_view_origin(core, pr_view.stdout)
  local origin = m_facts.pr_origin_fact(core, current_pr.comments)
  if origin == nil then
    origin = entity_lib.pr_native_origin(repo, pr_number, current_pr)
  end
  if origin.repo ~= repo or tostring(current_pr.head_ref_name or "") ~= tostring(origin.branch) then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|blocked", "skip-foreign(pr-origin)", "PR origin mismatch")
    return
  end
  if tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch)
    or tostring(origin.base_branch or "") ~= tostring(branches.integration) then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|blocked", "skip-foreign(base)", "PR base branch mismatch")
    return
  end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|blocked", "skip-stale(pr-closed)", "re-derived PR is not open")
    return
  end
  if tostring(current_pr.head_sha or "") ~= tostring(reviewed_head_sha) then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|blocked", "skip-stale(head-advanced)", "PR head advanced since unresolved review")
    return
  end
  if not m_claims.verify_pr_review_issue_claim(core, "review_loop", origin.repo, origin.issue_number, nil, origin.proposal_id) then
    return
  end

  local lock_key = entity_lib.transition_lock_key(origin.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|blocked", "skip-foreign(proposal_id)", "no issue transition lock key")
    return
  end
  local pr_source_ref = entity_lib.pr_source_ref(repo, pr_number)

  with_lock(lock_key, function()
    core.log_forged_markers("review_loop", origin.proposal_id, current_pr.comments)
    local state = require("devloop.entity").current_entity_state(core, current_pr.comments, origin.proposal_id)
    local transition = reviewing_segment_transition_status(state, review_version)
    if transition == "pending" then
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing|blocked", core.cas_outcome(state, "pending", review_version), "reviewing state marker not yet visible")
      error("github-devloop: reviewing marker not yet visible for review loop; retrying")
    end
    if transition == "stale" then
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing|blocked", "skip-stale(reviewing-version)", "issue is not currently reviewing at this version")
      return
    end
    local heartbeat_version = state.version
    local sr_digest = convergence_shared.source_ref_digest(unresolved.source_ref)
    local facts = conv_rounds.review_converge_round_facts(core, current_pr.comments, unresolved.proposal_id, origin.proposal_id, heartbeat_version, reviewed_head_sha, sr_digest)
    local round = math.max(tonumber(unresolved.round) or 0, conv_rounds.max_converge_round(core, facts))
    if conv_rounds.has_review_converge_round_marker(core, current_pr.comments, unresolved.proposal_id, origin.proposal_id, heartbeat_version, reviewed_head_sha, sr_digest, round) then
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing", "skip-idempotent(review converge round marker already visible)", "review converge round marker for incoming round is already visible")
      return
    end

    local marker_body = conv_rounds.review_converge_round_marker(core,
      unresolved.proposal_id,
      origin.proposal_id,
      heartbeat_version,
      reviewed_head_sha,
      sr_digest,
      round,
      unresolved.dedup_key,
      unresolved.narrowed_question,
      unresolved.angle_digests
    )
    local facts_with_current = conv_rounds.append_converge_round_fact(core, facts, round, unresolved.narrowed_question, unresolved.angle_digests, unresolved.dedup_key)
    local budget_round = math.max(round, conv_rounds.review_converge_budget_round(core, current_pr.comments, unresolved.proposal_id, origin.proposal_id))
    local hit_round_cap = budget_round >= config.max_converge_rounds(core)
    if hit_round_cap or conv_rounds.is_true_stall(core, facts_with_current, round) then
      local comment_request = requests_review.build_review_converge_round_comment_request(core, origin.repo, origin.issue_number, unresolved, origin.proposal_id, round, marker_body, pr_source_ref)
      local review_reconcile = conv_reconcile.build_devloop_review_reconcile_payload(core, unresolved, round, origin.proposal_id, review_version, reviewed_head_sha)
      local reason = hit_round_cap
        and ("PR review convergence budget reached at round " .. tostring(budget_round))
        or ("true PR review convergence stall at round " .. tostring(round))
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing", core.cas_outcome(state, transition, review_version), reason)
      core.log_apply("review_loop", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
        "github-proxy.github_pr_comment_request",
        "devloop_review_reconcile",
      })
      core.log_raise("review_loop", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
      core.log_raise("review_loop", origin.proposal_id, "devloop_review_reconcile", review_reconcile)
      return
    end
    if review_truth_table_unapproved(unresolved) then
      marker_body = marker_body .. "\n" .. core.state_marker(origin.proposal_id, "review-meta", state.version)
      local comment_request = requests_review.build_review_converge_round_comment_request(core, origin.repo, origin.issue_number, unresolved, origin.proposal_id, round, marker_body, pr_source_ref)
      local review_meta = payloads_builders.build_devloop_review_meta_payload(core, unresolved, origin.proposal_id, state.version, pr_number, round, pr_source_ref)
      local label_request = nil
      if origin.issue_number ~= nil then
        label_request = requests_labels.build_state_label_request(core, origin.repo, origin.issue_number, "review-meta", review_meta.dedup_key .. "/label/review-meta", pr_source_ref)
      end
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "review-meta", core.cas_outcome(state, transition, review_version), "review truth table reached no approve after bounded pass")
      core.log_apply("review_loop", origin.proposal_id, "review-meta", state.version, { add = { "fkst-dev:review-meta" }, remove = {} }, {
        "github-proxy.github_pr_comment_request",
        "github-proxy.github_issue_label_request",
        "devloop_review_meta",
      })
      core.log_raise("review_loop", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
      if label_request ~= nil then
        core.log_raise("review_loop", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
      end
      core.log_raise("review_loop", origin.proposal_id, "devloop_review_meta", review_meta)
      return
    end
    local comment_request = requests_review.build_review_converge_round_comment_request(core, origin.repo, origin.issue_number, unresolved, origin.proposal_id, round, marker_body, pr_source_ref)

    local current_issue = {
      title = "PR #" .. tostring(pr_number),
      body = "(PR-only review context; issue backing is absent)",
      comments = current_pr.comments,
    }
    if origin.issue_number ~= nil then
      local issue_view = core.gh_issue_view_review_loop(origin.repo, origin.issue_number, 30)
      if issue_view.exit_code ~= 0 then
        error("github-devloop: gh issue review loop view failed: " .. tostring(issue_view.stderr))
      end
      current_issue = parsers_issue.parse_issue_view_review_loop(core, issue_view.stdout)
    end
    local next_n = round + 1
    local next_dedup = conv_rounds.converge_proposal_base_dedup(core, unresolved.dedup_key) .. "/loop/" .. tostring(next_n)
    local context_fetch = { context_bundle.context_fetch_ref_from_bundle(core, {
      dept = "review_loop",
      repo = repo,
      issue_number = origin.issue_number,
      pr_number = pr_number,
      proposal_id = unresolved.proposal_id,
      version = next_dedup,
      tick = event.ts,
    }) }
    local content_fetch = context_fetch[1]
    local high_risk = context_fetch[2]
    local proposal = payloads_builders.build_board_pr_review_loop_proposal(core, repo, origin.issue_number, pr_number, state.version, current_pr.head_sha, current_issue, pr_source_ref, next_n, {
      narrowed_question = unresolved.narrowed_question,
      angle_digests = unresolved.angle_digests,
    }, event.ts, current_pr.comments, content_fetch, high_risk, next_dedup)
    if not v_validate_proposal.validate_proposal(core, proposal) then
      log.warn("github-devloop dept=review_loop proposal_id=" .. tostring(origin.proposal_id) .. " tag=SKIP reason=cannot-build-valid-review-loop-proposal")
      return
    end
    core.log_apply("review_loop", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
      "consensus.proposal",
      "github-proxy.github_pr_comment_request",
    })
    core.log_raise("review_loop", origin.proposal_id, "consensus.proposal", proposal)
    core.log_raise("review_loop", origin.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  end)
end, wrap = core.wrap_pipeline_failure, name = "review_loop" })
