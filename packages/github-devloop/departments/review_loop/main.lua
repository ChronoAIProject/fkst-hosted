local core = require("core")

local M = {}

M.spec = {
  consumes = { "consensus.consensus_converge" },
  produces = {
    "consensus.proposal",
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
    "devloop_review_meta",
  },
  fanout = { "consensus.consensus_converge" },
  stall_window = "30s",
  retry = { max_attempts = 12, base = "5s", cap = "30s" },
}

function pipeline(event)
  local unresolved = event.payload or {}
  if not core.is_supported_pr_review_unresolved(unresolved) then
    core.log_entry("review_loop", event, "unknown", unresolved.dedup_key)
    core.log_cas_decision("review_loop", "unknown", { state = nil, version = nil }, "reviewing", "reviewing|review-meta", "skip-foreign(proposal_id)", "unsupported event payload")
    return
  end

  core.log_entry("review_loop", event, unresolved.proposal_id, unresolved.dedup_key)
  local _, pr_number, review_version, reviewed_head_sha = core.parse_pr_review_proposal_id(unresolved.proposal_id)
  local repo, source_pr_number = core.parse_pr_source_ref(unresolved.source_ref)
  if repo == nil or tostring(source_pr_number) ~= tostring(pr_number) then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|review-meta", "skip-foreign(source_ref)", "review source_ref does not match PR review proposal")
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
      core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|review-meta", "retry-pending(pr-origin)", "trusted PR origin marker not yet visible")
      error("github-devloop: trusted pr-origin marker not yet visible for review loop; retrying")
    end
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|review-meta", "skip-foreign(pr-origin)", "trusted PR origin marker absent")
    return
  end
  if origin.repo ~= repo or tostring(current_pr.head_ref_name or "") ~= tostring(origin.branch) then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|review-meta", "skip-foreign(pr-origin)", "PR origin mismatch")
    return
  end
  if tostring(current_pr.base_ref_name or "") ~= tostring(origin.base_branch)
    or tostring(origin.base_branch or "") ~= tostring(branches.integration) then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|review-meta", "skip-foreign(base)", "PR base branch mismatch")
    return
  end
  if tostring(current_pr.state or ""):lower() ~= "open" then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|review-meta", "skip-stale(pr-closed)", "re-derived PR is not open")
    return
  end
  if tostring(current_pr.head_sha or "") ~= tostring(reviewed_head_sha) then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|review-meta", "skip-stale(head-advanced)", "PR head advanced since unresolved review")
    return
  end

  local lock_key = core.transition_lock_key(origin.proposal_id)
  if lock_key == nil then
    core.log_cas_decision("review_loop", unresolved.proposal_id, { state = nil, version = nil }, "reviewing", "reviewing|review-meta", "skip-foreign(proposal_id)", "no issue transition lock key")
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
    local transition = core.cyclic_transition_status(state, { "reviewing" }, "review-meta", review_version)
    if transition == "pending" then
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing|review-meta", core.cas_outcome(state, transition, review_version), "reviewing state marker not yet visible")
      error("github-devloop: reviewing marker not yet visible for review loop; retrying")
    end
    if state.state ~= "reviewing" or transition == "stale" then
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing|review-meta", core.cas_outcome(state, transition, review_version), "issue is not currently reviewing")
      return
    end
    if tostring(core.safe_version_segment(state.version or "")) ~= tostring(review_version) then
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing|review-meta", "skip-stale(version-mismatch)", "review proposal version does not match canonical issue marker")
      return
    end
    if core.has_review_loop_marker_dedup(current_issue.comments, unresolved.proposal_id, origin.proposal_id, unresolved.dedup_key) then
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing|review-meta", "skip-idempotent(already handled unresolved dedup)", "review loop marker for incoming dedup is already visible")
      return
    end

    local budget = core.loop_budget()
    local event_n = core.parse_loop_round_from_dedup(unresolved.dedup_key)
    local marker_n = core.review_loop_count_from_github_markers(current_issue.comments, unresolved.proposal_id, origin.proposal_id)
    local current_n = math.max(event_n, marker_n)
    if current_n >= budget then
      local review_meta_version = tostring(state.version) .. "/review-loop/" .. tostring(current_n)
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "review-meta", core.cas_outcome(state, transition, review_version), "review loop budget exhausted at round " .. tostring(current_n))
      local comment_request = core.build_review_meta_trigger_comment_request(origin.repo, origin.issue_number, unresolved, origin.proposal_id, review_meta_version, budget, issue_source_ref)
      local label_request = core.build_review_meta_trigger_label_request(origin.repo, origin.issue_number, unresolved, origin.proposal_id, budget, issue_source_ref)
      local add_labels, remove_labels = core.state_label_changes("review-meta")
      core.log_apply("review_loop", origin.proposal_id, "review-meta", review_meta_version, { add = add_labels, remove = remove_labels }, {
        "github-proxy.github_issue_comment_request",
        "github-proxy.github_issue_label_request",
        "devloop_review_meta",
      })
      core.log_raise("review_loop", origin.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
      core.log_raise("review_loop", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
      core.log_raise("review_loop", origin.proposal_id, "devloop_review_meta", core.build_devloop_review_meta_payload(unresolved, origin.proposal_id, review_meta_version, pr_number, current_n, issue_source_ref))
      return
    end

    local next_n = current_n + 1
    if core.has_review_loop_marker_round(current_issue.comments, unresolved.proposal_id, origin.proposal_id, next_n) then
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "reviewing|review-meta", "skip-idempotent(already handled round)", "review loop marker for next round is already visible")
      return
    end
    if next_n >= budget then
      local review_meta_version = tostring(state.version) .. "/review-loop/" .. tostring(next_n)
      core.log_cas_decision("review_loop", origin.proposal_id, state, "reviewing", "review-meta", core.cas_outcome(state, transition, review_version), "next review loop round reaches budget " .. tostring(next_n))
      local comment_request = core.build_review_meta_trigger_comment_request(origin.repo, origin.issue_number, unresolved, origin.proposal_id, review_meta_version, next_n, issue_source_ref)
      local label_request = core.build_review_meta_trigger_label_request(origin.repo, origin.issue_number, unresolved, origin.proposal_id, next_n, issue_source_ref)
      local add_labels, remove_labels = core.state_label_changes("review-meta")
      core.log_apply("review_loop", origin.proposal_id, "review-meta", review_meta_version, { add = add_labels, remove = remove_labels }, {
        "github-proxy.github_issue_comment_request",
        "github-proxy.github_issue_label_request",
        "devloop_review_meta",
      })
      core.log_raise("review_loop", origin.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
      core.log_raise("review_loop", origin.proposal_id, "github-proxy.github_issue_label_request", label_request)
      core.log_raise("review_loop", origin.proposal_id, "devloop_review_meta", core.build_devloop_review_meta_payload(unresolved, origin.proposal_id, review_meta_version, pr_number, next_n, issue_source_ref))
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
    local proposal = core.build_pr_review_loop_proposal(repo, origin.issue_number, pr_number, state.version, current_pr.head_sha, current_issue, diff.stdout, pr_source_ref, next_n)
    if not core.validate_proposal(proposal) then
      log.warn("github-devloop dept=review_loop proposal_id=" .. tostring(origin.proposal_id) .. " tag=SKIP reason=cannot-build-valid-review-loop-proposal")
      return
    end
    local comment_request = core.build_review_loop_comment_request(origin.repo, origin.issue_number, unresolved, origin.proposal_id, next_n, issue_source_ref)
    core.log_apply("review_loop", origin.proposal_id, nil, nil, { add = {}, remove = {} }, {
      "consensus.proposal",
      "github-proxy.github_issue_comment_request",
    })
    core.log_raise("review_loop", origin.proposal_id, "consensus.proposal", proposal)
    core.log_raise("review_loop", origin.proposal_id, "github-proxy.github_issue_comment_request", comment_request)
  end)
end

return M
