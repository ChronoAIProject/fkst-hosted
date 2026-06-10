local core = require("core")

local M = {}

M.spec = {
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
    origin = core.pr_native_origin(repo, pr_number, current_pr)
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
  local pr_source_ref = core.pr_source_ref(repo, pr_number)

  with_lock(lock_key, function()
    core.log_forged_markers("review_loop", origin.proposal_id, current_pr.comments)
    local state = core.current_entity_state(current_pr.comments, origin.proposal_id)
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
    local facts = core.review_converge_round_facts(current_pr.comments, unresolved.proposal_id, origin.proposal_id, review_version, reviewed_head_sha, sr_digest)
    local round = math.max(tonumber(unresolved.round) or 0, core.max_converge_round(facts))
    if core.has_review_converge_round_marker(current_pr.comments, unresolved.proposal_id, origin.proposal_id, review_version, reviewed_head_sha, sr_digest, round) then
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
    local facts_with_current = append_round_fact(facts, round, unresolved.narrowed_question, unresolved.angle_digests, unresolved.dedup_key)
    local hit_round_cap = round >= core.max_converge_rounds()
    if hit_round_cap or core.is_true_stall(facts_with_current, round) then
      local comment_request = core.build_review_converge_round_comment_request(origin.repo, origin.issue_number, unresolved, origin.proposal_id, round, marker_body, pr_source_ref)
      local review_reconcile = core.build_devloop_review_reconcile_payload(unresolved, round, origin.proposal_id, review_version, reviewed_head_sha)
      local reason = hit_round_cap
        and ("PR review convergence round cap reached at round " .. tostring(round))
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
      local comment_request = core.build_review_converge_round_comment_request(origin.repo, origin.issue_number, unresolved, origin.proposal_id, round, marker_body, pr_source_ref)
      local review_meta = core.build_devloop_review_meta_payload(unresolved, origin.proposal_id, state.version, pr_number, round, pr_source_ref)
      local label_request = nil
      if origin.issue_number ~= nil then
        label_request = core.build_state_label_request(origin.repo, origin.issue_number, "review-meta", review_meta.dedup_key .. "/label/review-meta", pr_source_ref)
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
    local comment_request = core.build_review_converge_round_comment_request(origin.repo, origin.issue_number, unresolved, origin.proposal_id, round, marker_body, pr_source_ref)

    local current_issue = {
      title = "PR #" .. tostring(pr_number),
      body = "(PR-only review context; issue backing is absent)",
      comments = current_pr.comments,
    }
    if origin.issue_number ~= nil then
      local issue_view = exec_sync({ cmd = core.gh_issue_view_review_loop_cmd(origin.repo, origin.issue_number), timeout = 30 })
      if issue_view.exit_code ~= 0 then
        error("github-devloop: gh issue review loop view failed: " .. tostring(issue_view.stderr))
      end
      current_issue = core.parse_issue_view_review_loop(issue_view.stdout)
    end
    local next_n = round + 1
    local proposal = core.build_board_pr_review_loop_proposal(repo, origin.issue_number, pr_number, state.version, current_pr.head_sha, current_issue, pr_source_ref, next_n, {
      narrowed_question = unresolved.narrowed_question,
      angle_digests = unresolved.angle_digests,
    }, event.ts, current_pr.comments)
    if not core.validate_proposal(proposal) then
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
end

return M
