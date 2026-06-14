local S = {}

function S.install(M)
function M.build_pr_state_label_request(repo, issue_number, pr_number, proposal_id, to_state, version, dedup_key_value, source_ref, current_labels)
  local add_labels, remove_labels
  if current_labels ~= nil then
    add_labels, remove_labels = M.state_label_reconcile_changes(current_labels, to_state)
  else
    add_labels, remove_labels = M.state_label_changes(to_state)
  end
  return M.attach_issue_claim({
    schema = "github-proxy.label.v1",
    repo = repo,
    target_kind = "pr",
    target_number = pr_number,
    pr_number = pr_number,
    issue_number = issue_number,
    expected_proposal_id = proposal_id,
    expected_state = to_state,
    expected_version = version,
    add_labels = add_labels,
    remove_labels = remove_labels,
    dedup_key = dedup_key_value,
    source_ref = M.normalize_source_ref(source_ref),
  }, issue_number ~= nil and M.issue_source_ref(repo, issue_number) or nil)
end

function M.build_reconcile_pr_state_label_request(repo, issue_number, pr_number, proposal_id, state, version, source_ref, current_labels)
  return M.build_pr_state_label_request(
    repo,
    issue_number,
    pr_number,
    proposal_id,
    state,
    version,
    M._dedup_key({
      "reconcile",
      "pr-label",
      tostring(proposal_id),
      tostring(state),
      tostring(version or "unversioned"),
      tostring(pr_number),
    }),
    source_ref,
    current_labels
  )
end

function M.build_pr_reviewing_label_request(repo, issue_number, origin, pr_number, source_ref)
  return M.build_pr_state_label_request(
    repo,
    issue_number,
    pr_number,
    origin.proposal_id,
    "reviewing",
    origin.impl_version,
    M._dedup_key({
      "observe-pr",
      "pr-label",
      tostring(origin.proposal_id),
      tostring(origin.impl_version),
      tostring(pr_number),
    }),
    source_ref
  )
end

end

return S
