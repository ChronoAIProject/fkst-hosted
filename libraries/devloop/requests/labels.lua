local S = {}

function S.install(M, shared)
function M.build_label_request(repo, issue_number, add_labels, remove_labels, dedup_key, source_ref)
  return M.attach_issue_claim({
    schema = "github-proxy.label.v1",
    repo = repo,
    target_kind = "issue",
    target_number = issue_number,
    issue_number = issue_number,
    add_labels = add_labels or {},
    remove_labels = remove_labels or {},
    dedup_key = dedup_key,
    source_ref = M.normalize_source_ref(source_ref),
  }, source_ref)
end

function M.build_state_label_request(repo, issue_number, to_state, dedup_key_value, source_ref)
  local add_labels, remove_labels = M.state_label_changes(to_state)
  return M.build_label_request(repo, issue_number, add_labels, remove_labels, dedup_key_value, source_ref)
end

function M.build_thinking_label_request(issue, proposal)
  return M.build_state_label_request(
    issue.repo,
    issue.number,
    "thinking",
    tostring(proposal.effect_version or proposal.dedup_key) .. "/label/thinking",
    issue.source_ref
  )
end

function M.build_result_label_request(repo, issue_number, reached)
  return M.build_state_label_request(
    repo,
    issue_number,
    "ready",
    tostring(reached.proposal_id) .. "/label/" .. tostring(reached.decision),
    reached.source_ref
  )
end

function M.build_intake_enabled_label_request(repo, issue_number, candidate)
  local add_labels, remove_labels = M.intake_service_class_label_changes(candidate.service_class)
  table.insert(add_labels, 1, M._enabled_label)
  return M.build_label_request(
    repo,
    issue_number,
    add_labels,
    remove_labels,
    M._dedup_key({
      "intake",
      "label",
      tostring(candidate.proposal_id),
      tostring(candidate.dedup_key),
    }),
    candidate.source_ref
  )
end

function M.build_intake_tracking_label_request(repo, issue_number, candidate)
  local add_labels, remove_labels = M.intake_service_class_label_changes(candidate.service_class)
  table.insert(add_labels, 1, M._tracking_label)
  return M.build_label_request(
    repo,
    issue_number,
    add_labels,
    remove_labels,
    M._dedup_key({
      "intake",
      "label",
      "tracking",
      tostring(candidate.proposal_id),
      tostring(candidate.dedup_key),
    }),
    candidate.source_ref
  )
end

function M.build_implementing_label_request(repo, issue_number, ready)
  return M.build_state_label_request(
    repo,
    issue_number,
    "implementing",
    M._dedup_key({
      "implement",
      "label",
      "implementing",
      tostring(ready.dedup_key),
    }),
    ready.source_ref
  )
end

function M.build_impl_failed_label_request(repo, issue_number, ready, reason)
  return M.build_state_label_request(
    repo,
    issue_number,
    "impl-failed",
    M._dedup_key({
      "implement",
      "label",
      "impl-failed",
      tostring(reason or "failed"),
      tostring(ready.dedup_key),
    }),
    ready.source_ref
  )
end

function M.build_reviewing_label_request(repo, issue_number, origin, pr_number, source_ref)
  return M.build_state_label_request(
    repo,
    issue_number,
    "reviewing",
    M._dedup_key({
      "observe-pr",
      "label",
      tostring(origin.proposal_id),
      tostring(origin.impl_version),
      tostring(pr_number),
    }),
    source_ref
  )
end

function M.build_pr_base_unmanaged_label_request(repo, issue_number, origin, pr_number, integration_branch, source_ref)
  return M.build_state_label_request(
    repo,
    issue_number,
    "blocked",
    M._dedup_key({
      "observe-pr",
      "label",
      "pr-base-unmanaged",
      tostring(origin.proposal_id),
      tostring(origin.impl_version),
      tostring(pr_number),
      tostring(origin.base_branch),
      tostring(integration_branch),
    }),
    source_ref
  )
end

function M.build_review_result_label_request(repo, issue_number, issue_proposal_id, reached, source_ref)
  local to_state = reached.reflection_checkpoint and "review-meta"
    or reached.decision == "approve" and "merge-ready"
    or "fixing"
  return M.build_state_label_request(
    repo,
    issue_number,
    to_state,
    M._dedup_key({
      "review-result",
      "label",
      tostring(issue_proposal_id),
      tostring(reached.decision),
      tostring(reached.dedup_key),
    }),
    source_ref
  )
end

function M.build_fix_reviewing_label_request(repo, issue_number, fix, new_head_sha, new_version)
  return M.build_state_label_request(
    repo,
    issue_number,
    "reviewing",
    M._dedup_key({
      "fix",
      "label",
      tostring(fix.proposal_id),
      tostring(fix.review_dedup_key),
      tostring(new_head_sha),
    }),
    fix.source_ref
  )
end

function M.build_merge_head_reviewing_label_request(repo, issue_number, merge_ready, new_head_sha, new_version, source_ref)
  return M.build_state_label_request(
    repo,
    issue_number,
    "reviewing",
    M._dedup_key({
      "merge",
      "label",
      "reviewing",
      tostring(merge_ready.proposal_id),
      tostring(new_version),
      tostring(new_head_sha),
    }),
    source_ref
  )
end
end

return S
