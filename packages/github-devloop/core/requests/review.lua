local S = {}

function S.install(M, shared)
local ai_sentinel = shared.ai_sentinel
local build_convergence_display = shared.build_convergence_display
local build_verdict_summary = shared.build_verdict_summary
local bounded_blocking_gap = shared.bounded_blocking_gap

function M.build_review_converge_round_comment_request(repo, issue_number, unresolved, issue_proposal_id, round, marker_body, source_ref)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = unresolved.pr_number or select(2, M.parse_pr_source_ref(unresolved.source_ref)),
  }, build_convergence_display(M.comment_string("pr_review_convergence_round_prefix"), unresolved, round)
    .. "\n\n" .. tostring(marker_body)
    .. "\n" .. ai_sentinel, M._dedup_key({
    "review-converge-round",
    "comment",
    tostring(issue_proposal_id),
    tostring(round),
    tostring(unresolved.dedup_key),
  }), source_ref or unresolved.source_ref)
end

function M.build_issue_review_converge_round_comment_request(repo, issue_number, unresolved, issue_proposal_id, round, marker_body, source_ref)
  return M.attach_issue_claim({
    schema = "github-proxy.v1",
    repo = repo,
    issue_number = issue_number,
    body = build_convergence_display(M.comment_string("pr_review_convergence_round_prefix"), unresolved, round)
      .. "\n\n" .. tostring(marker_body)
      .. "\n" .. ai_sentinel,
    dedup_key = M._dedup_key({
      "review-converge-round",
      "comment",
      tostring(issue_proposal_id),
      tostring(round),
      tostring(unresolved.dedup_key),
    }),
    source_ref = M.normalize_source_ref(source_ref or unresolved.source_ref),
  }, source_ref or unresolved.source_ref)
end

function M.build_reviewing_comment_request(repo, issue_number, origin, pr_number, source_ref)
  local state_marker = M.state_marker(origin.proposal_id, "reviewing", origin.impl_version)
  local request = M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, M.comment_string("pr_ready_for_review")
    .. "\n\n" .. state_marker, M._dedup_key({
    "observe-pr",
    "comment",
    tostring(origin.proposal_id),
    tostring(origin.impl_version),
    tostring(pr_number),
  }), source_ref)
  request.handoff = {
    kind = "github-devloop.reviewing",
    proposal_id = origin.proposal_id,
    pr_number = pr_number,
    version = origin.impl_version,
    source_ref = M.normalize_source_ref(source_ref),
  }
  return request
end

function M.pr_base_unmanaged_blocked_version(version)
  return tostring(version or "") .. "/blocked/pr-base-unmanaged"
end

function M.build_pr_base_unmanaged_comment_request(repo, pr_number, origin, integration_branch, source_ref)
  local blocked_version = M.pr_base_unmanaged_blocked_version(origin.impl_version)
  local state_marker = M.state_marker(origin.proposal_id, "blocked", blocked_version)
  local reason_marker = M.pr_base_unmanaged_marker(origin.proposal_id, pr_number, origin.base_branch, integration_branch)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, "github-devloop blocked PR because its base branch is not managed by this instance."
    .. "\n\nReason: pr-base-unmanaged"
    .. "\nPR base: " .. tostring(origin.base_branch)
    .. "\nConfigured integration branch: " .. tostring(integration_branch)
    .. "\n\n" .. state_marker
    .. "\n" .. reason_marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "observe-pr",
    "blocked",
    "pr-base-unmanaged",
    tostring(origin.proposal_id),
    tostring(origin.impl_version),
    tostring(pr_number),
    tostring(origin.base_branch),
    tostring(integration_branch),
  }), source_ref)
end

function M.build_review_result_comment_request(repo, issue_number, issue_proposal_id, issue_version, reached, source_ref)
  local to_state = reached.reflection_checkpoint and "review-meta"
    or reached.decision == "approve" and "merge-ready"
    or "fixing"
  local state_marker = M.state_marker(issue_proposal_id, to_state, issue_version)
  local fix_round = nil
  if reached.decision == "reject" then
    fix_round = M.version_fix_round(issue_version)
  end
  local blocking_gap = bounded_blocking_gap(M, reached)
  local marker = M.review_result_marker(reached.proposal_id, issue_proposal_id, reached.decision, reached.dedup_key, fix_round, blocking_gap)
  local reflection_marker = ""
  if reached.reflection_checkpoint then
    reflection_marker = "\n" .. M.fix_reflection_marker(issue_proposal_id, reached.dedup_key, "checkpoint", issue_version, fix_round)
  end
  local merge_marker = ""
  if reached.decision == "approve" then
    local _, pr_number, _, reviewed_head_sha = M.parse_pr_review_proposal_id(reached.proposal_id)
    merge_marker = "\n" .. M.merge_ready_marker(issue_proposal_id, pr_number, issue_version, reached.proposal_id, reached.dedup_key, reviewed_head_sha)
  end
  local body_text = M.neutralize_untrusted_comment_text(reached.body or "")
  local verdict_summary = build_verdict_summary(reached.angle_results)
  local body = M.comment_string("pr_review_decision_prefix") .. tostring(reached.decision)
  if verdict_summary ~= nil then
    body = body .. "\n" .. verdict_summary
  end
  if reached.decision == "reject" and blocking_gap ~= nil then
    body = body .. "\n" .. M.comment_string("blocking_gap_label") .. M.neutralize_untrusted_comment_text(blocking_gap)
  end
  local _, pr_number = M.parse_pr_source_ref(source_ref)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, body
    .. "\n\n" .. body_text
    .. "\n\n" .. state_marker
    .. "\n" .. marker
    .. reflection_marker
    .. merge_marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "review-result",
    "comment",
    tostring(issue_proposal_id),
    tostring(reached.decision),
    tostring(reached.dedup_key),
  }), source_ref)
end

function M.build_merge_gate_fix_comment_request(repo, issue_number, merge_ready, fix_version, reason, gate_baseline_sha, source_ref, predecessor_set)
  local safe_reason = M.merge_gate_reason_class(reason)
  local display_reason = M.neutralize_untrusted_comment_text(reason or "gate-failed")
  if display_reason == "" then
    display_reason = "gate-failed"
  end
  if gate_baseline_sha ~= nil and not M._is_git_sha(gate_baseline_sha) then
    error("github-devloop: invalid merge-gate baseline sha")
  end
  local test_command = M.neutralize_untrusted_comment_text(M.test_command())
  local state_marker = M.state_marker(merge_ready.proposal_id, "fixing", fix_version)
  local marker = M.merge_gate_marker(
    merge_ready.proposal_id,
    merge_ready.pr_number,
    fix_version,
    merge_ready.review_proposal_id,
    merge_ready.review_dedup_key,
    merge_ready.reviewed_head_sha,
    gate_baseline_sha,
    safe_reason,
    predecessor_set
  )
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = merge_ready.pr_number,
  }, M.comment_string("merge_gate_failed_prefix") .. display_reason
    .. "\n" .. M.comment_string("reproduce_locally_prefix") .. test_command .. M.comment_string("reproduce_locally_suffix")
    .. "\n\n" .. state_marker
    .. "\n" .. marker, M._dedup_key({
    "merge",
    "comment",
    "fixing",
    tostring(merge_ready.proposal_id),
    tostring(merge_ready.version),
    tostring(fix_version),
    tostring(predecessor_set or "nopred"),
    safe_reason,
  }), source_ref)
end

function M.build_fix_reviewing_comment_request(repo, issue_number, fix, old_head_sha, new_head_sha, new_version)
  local state_marker = M.state_marker(fix.proposal_id, "reviewing", new_version or fix.version)
  local marker = M.fix_marker(fix.proposal_id, fix.review_proposal_id, fix.review_dedup_key, old_head_sha, new_head_sha)
  local summary = ""
  if fix.fix_summary ~= nil and tostring(fix.fix_summary) ~= "" then
    summary = "\n" .. M.comment_string("fix_round_summary_label") .. M.neutralize_untrusted_comment_text(fix.fix_summary)
  end
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = fix.pr_number,
  }, M.comment_string("fix_pushed_for_rereview")
    .. "\n\n" .. M.comment_string("previous_reviewed_head_label") .. tostring(old_head_sha)
    .. "\n" .. M.comment_string("new_head_label") .. tostring(new_head_sha)
    .. summary
    .. "\n\n" .. state_marker
    .. "\n" .. marker, M._dedup_key({
    "fix",
    "comment",
    tostring(fix.proposal_id),
    tostring(fix.review_dedup_key),
    tostring(new_head_sha),
  }), fix.source_ref)
end

function M.raise_fix_reviewing(opts)
  opts = opts or {}
  local dept = tostring(opts.dept or "unknown")
  local repo = opts.repo
  local issue_number = opts.issue_number
  local fix = opts.fix or {}
  local old_head_sha = opts.old_head_sha
  local new_head_sha = opts.new_head_sha
  local new_version = opts.new_version or M.next_fix_version(fix.version)
  local reason = opts.reason
  local current_state = opts.current_state or { state = "fixing", version = fix.version }
  if opts.fix_summary ~= nil or opts.clear_fix_summary == true then
    fix.fix_summary = opts.fix_summary
  end

  M.log_cas_decision(dept, fix.proposal_id, current_state, "fixing", "reviewing", "applied", reason)
  local comment_request = M.build_fix_reviewing_comment_request(repo, issue_number, fix, old_head_sha, new_head_sha, new_version)
  local label_request = M.build_fix_reviewing_label_request(repo, issue_number, fix, new_head_sha, new_version)
  local add_labels, remove_labels = M.state_label_changes("reviewing")
  local reviewing_payload = M.build_devloop_reviewing_payload({
    proposal_id = fix.proposal_id,
    impl_version = new_version,
  }, fix.pr_number, fix.source_ref)
  M.log_apply(dept, fix.proposal_id, "reviewing", new_version, { add = add_labels, remove = remove_labels }, {
    "github-proxy.github_pr_comment_request",
    "github-proxy.github_issue_label_request",
    "devloop_reviewing",
  })
  M.log_raise(dept, fix.proposal_id, "github-proxy.github_pr_comment_request", comment_request)
  if issue_number ~= nil then
    M.log_raise(dept, fix.proposal_id, "github-proxy.github_issue_label_request", label_request)
  end
  M.log_raise(dept, fix.proposal_id, "devloop_reviewing", reviewing_payload)
end

function M.build_merge_head_reviewing_comment_request(repo, issue_number, merge_ready, old_head_sha, new_head_sha, new_version, source_ref)
  local state_marker = M.state_marker(merge_ready.proposal_id, "reviewing", new_version)
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = merge_ready.pr_number,
  }, M.comment_string("pr_head_advanced")
    .. "\n\n" .. M.comment_string("previous_reviewed_head_label") .. tostring(old_head_sha)
    .. "\n" .. M.comment_string("current_head_label") .. tostring(new_head_sha)
    .. "\n\n" .. state_marker, M._dedup_key({
    "merge",
    "comment",
    "reviewing",
    tostring(merge_ready.proposal_id),
    tostring(new_version),
    tostring(new_head_sha),
  }), source_ref)
end

function M.build_review_carry_over_comment_request(repo, pr_number, issue_proposal_id, version, carry, source_ref)
  local state_marker = M.state_marker(issue_proposal_id, "merge-ready", version)
  local review_marker = M.review_result_marker(carry.new_review_proposal_id, issue_proposal_id, "approve", carry.new_review_dedup_key)
  local merge_marker = M.merge_ready_marker(issue_proposal_id, pr_number, version, carry.new_review_proposal_id, carry.new_review_dedup_key, carry.new_head_sha)
  local carry_marker = M.review_carry_over_marker(
    issue_proposal_id,
    version,
    carry.old_review_proposal_id,
    carry.old_review_dedup_key,
    carry.approved_head_sha,
    carry.new_review_proposal_id,
    carry.new_review_dedup_key,
    carry.new_head_sha,
    carry.base_head_sha
  )
  return M.build_entity_comment_request({
    kind = "pr",
    repo = repo,
    number = pr_number,
  }, "github-devloop PR review approval carried over"
    .. "\nResolution delta proof: merge-tree-empty-delta"
    .. "\nApproved head: " .. tostring(carry.approved_head_sha)
    .. "\nNew head: " .. tostring(carry.new_head_sha)
    .. "\nBase head: " .. tostring(carry.base_head_sha)
    .. "\n\n" .. state_marker
    .. "\n" .. review_marker
    .. "\n" .. merge_marker
    .. "\n" .. carry_marker
    .. "\n" .. ai_sentinel, M._dedup_key({
    "review-carry-over",
    "comment",
    tostring(issue_proposal_id),
    tostring(version),
    tostring(carry.approved_head_sha),
    tostring(carry.new_head_sha),
  }), source_ref)
end
end

return S
