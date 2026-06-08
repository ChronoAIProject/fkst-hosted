local core = require("core")

local M = {}

M.spec = {
  consumes = { "consensus.consensus_converge" },
  produces = {
    "consensus.proposal",
    "github-proxy.github_issue_comment_request",
    "devloop_review_reconcile",
  },
  fanout = { "consensus.consensus_converge" },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

local function append_round_fact(facts, round, narrowed_question, angle_digests, dedup_key)
  local copied = {}
  for _, fact in ipairs(facts or {}) do
    table.insert(copied, fact)
  end
  table.insert(copied, {
    round = round,
    question = core.converge_question_digest(narrowed_question),
    verdicts = core.converge_verdicts_digest(angle_digests),
    dedup = dedup_key,
  })
  return copied
end

-- review_version is parse_pr_review_proposal_id's safe_version_segment form (truncated +
-- checksummed for long versions): it preserves version EQUALITY only, never ordering, so it
-- must NOT be fed to the ordering-based CAS. The PR head is already pinned to reviewed_head_sha
-- before the lock, so we only need: same reviewing version (segment equality) -> apply; issue
-- advanced past reviewing (stage_rank, which IS order-preserving) or reviewing at a different
-- version -> stale skip; not yet at reviewing -> pending retry.
local function reviewing_segment_transition_status(state, review_version)
  if state.state == "reviewing"
    and tostring(core.safe_version_segment(state.version or "")) == tostring(review_version) then
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

function pipeline(event)
  local unresolved = event.payload or {}
  if not core.is_supported_pr_review_unresolved(unresolved) then
    core.log_entry("review_loop", event, "unknown", unresolved.dedup_key)
    core.log_cas_decision("review_loop", "unknown", { state = nil, version = nil }, "reviewing", "reviewing|blocked", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("review_loop", event, unresolved.proposal_id, unresolved.dedup_key)
  local _, pr_number, review_version, reviewed_head_sha = core.parse_pr_review_proposal_id(unresolved.proposal_id)
  local repo, source_pr_number = core.parse_pr_source_ref(unresolved.source_ref)
  if repo == nil or tostring(source_pr_number) ~= tostring(pr_number) then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|blocked", "skip-foreign(source_ref)", "review source_ref does not match PR review proposal")
    return
  end

  core.assert_trusted_bot_configured()
  local branches = core.branch_config()
  local pr_view = exec_sync({ cmd = core.gh_pr_view_origin_cmd(repo, pr_number), timeout = 30 })
  if pr_view.exit_code ~= 0 then
    error("github-devloop: gh pr origin view failed for review loop: " .. tostring(pr_view.stderr))
  end
  local current_pr = core.parse_pr_view_origin(pr_view.stdout)
  local origin = core.pr_origin_fact(current_pr.comments)
  if origin == nil then
    if core.is_devloop_issue_branch(current_pr.head_ref_name) then
      core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|blocked", "retry-pending(pr-origin)", "trusted PR origin marker not yet visible")
      error("github-devloop: trusted pr-origin marker not yet visible for review loop; retrying")
    end
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|blocked", "skip-foreign(pr-origin)", "trusted PR origin marker absent")
    return
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

  local lock_key = core.transition_lock_key(origin.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|blocked", "skip-foreign(proposal_id)", "no issue transition lock key")
    return
  end
  local issue_source_ref = {
    kind = "external",
    ref = tostring(origin.repo) .. "#issue/" .. tostring(origin.issue_number),
  }

  with_lock(lock_key, function()
    local issue_view = exec_sync({ cmd = core.gh_issue_view_review_loop_cmd(origin.repo, origin.issue_number), timeout = 30 })
    if issue_view.exit_code ~= 0 then
      error("github-devloop: gh issue review loop view failed: " .. tostring(issue_view.stderr))
    end
    local current_issue = core.parse_issue_view_review_loop(issue_view.stdout)
    core.log_forged_markers("review_loop", origin.proposal_id, current_issue.comments)
    local state = core.current_state(current_issue.comments, origin.proposal_id)
    local transition = reviewing_segment_transition_status(state, review_version)
    if transition == "pending" then
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing|blocked", core.cas_outcome(state, "pending", review_version), "reviewing state marker not yet visible")
      error("github-devloop: reviewing marker not yet visible for review loop; retrying")
    end
    if transition == "stale" then
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing|blocked", "skip-stale(reviewing-version)", "issue is not currently reviewing at this version")
      return
    end
    local sr_digest = core.source_ref_digest(unresolved.source_ref)
    local facts = core.review_converge_round_facts(current_issue.comments, unresolved.proposal_id, origin.proposal_id, review_version, reviewed_head_sha, sr_digest)
    local round = math.max(tonumber(unresolved.round) or 0, core.max_converge_round(facts))
    if core.has_review_converge_round_marker(current_issue.comments, unresolved.proposal_id, origin.proposal_id, review_version, reviewed_head_sha, sr_digest, round) then
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing", "skip-idempotent(review converge round marker already visible)", "review converge round marker for incoming round is already visible")
      return
    end

    local marker_body = core.review_converge_round_marker(
      unresolved.proposal_id,
      origin.proposal_id,
      review_version,
      reviewed_head_sha,
      sr_digest,
      round,
      unresolved.dedup_key,
      unresolved.narrowed_question,
      unresolved.angle_digests
    )
    local comment_request = core.build_review_converge_round_comment_request(origin.repo, origin.issue_number, unresolved, origin.proposal_id, round, marker_body, issue_source_ref)
    local facts_with_current = append_round_fact(facts, round, unresolved.narrowed_question, unresolved.angle_digests, unresolved.dedup_key)
    if core.is_true_stall(facts_with_current, round) then
      local review_reconcile = core.build_devloop_review_reconcile_payload(unresolved, round, origin.proposal_id, review_version, reviewed_head_sha)
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing", core.cas_outcome(state, transition, review_version), "true PR review convergence stall at round " .. tostring(round))
      core.log_apply("review_loop", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
        "github-proxy.github_issue_comment_request",
        "devloop_review_reconcile",
      })
      core.log_raise("review_loop", origin.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
      core.log_raise("review_loop", origin.proposal_id, "devloop_review_reconcile", review_reconcile)
      return
    end

    local diff = exec_sync({ cmd = core.gh_pr_diff_cmd(repo, pr_number), timeout = 30 })
    if diff.exit_code ~= 0 then
      error("github-devloop: gh pr diff failed for review loop: " .. tostring(diff.stderr))
    end
    local after_diff_pr_view = exec_sync({ cmd = core.gh_pr_view_origin_cmd(repo, pr_number), timeout = 30 })
    if after_diff_pr_view.exit_code ~= 0 then
      error("github-devloop: gh pr review loop head recheck failed: " .. tostring(after_diff_pr_view.stderr))
    end
    local after_diff_pr = core.parse_pr_view_origin(after_diff_pr_view.stdout)
    if tostring(after_diff_pr.head_ref_name or "") ~= tostring(current_pr.head_ref_name or "")
      or tostring(after_diff_pr.head_sha or "") ~= tostring(current_pr.head_sha or "")
      or tostring(after_diff_pr.state or ""):lower() ~= "open" then
      error("github-devloop: PR head moved while reading review loop diff; retrying")
    end

    local pr_source_ref = {
      kind = "external",
      ref = tostring(repo) .. "#pr/" .. tostring(pr_number),
    }
    local next_n = round + 1
    local proposal = core.build_pr_review_loop_proposal(repo, origin.issue_number, pr_number, state.version, current_pr.head_sha, current_issue, diff.stdout, pr_source_ref, next_n, {
      narrowed_question = unresolved.narrowed_question,
      angle_digests = unresolved.angle_digests,
    })
    if not core.validate_proposal(proposal) then
      log.warn("github-devloop dept=review_loop proposal_id=" .. tostring(origin.proposal_id) .. " tag=SKIP reason=cannot-build-valid-review-loop-proposal")
      return
    end
    core.log_apply("review_loop", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
      "consensus.proposal",
      "github-proxy.github_issue_comment_request",
    })
    core.log_raise("review_loop", origin.proposal_id, "consensus.proposal", proposal)
    core.log_raise("review_loop", origin.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  end)
end

return M
