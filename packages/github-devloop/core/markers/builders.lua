local S = {}

function S.install(M, shared)
local valid_round = shared.valid_round
local strings = shared.strings
local safe_marker_attr = shared.safe_marker_attr

function M.review_meta_marker(issue_proposal_id, dedup_key, action, version, blocking_gap, reason)
  local fields = ""
  if action ~= nil then
    if not M._is_review_meta_action(action) then
      error("github-devloop: invalid review-meta action")
    end
    fields = fields .. '" action="' .. tostring(action)
  end
  if version ~= nil then
    fields = fields .. '" version="' .. tostring(version)
  end
  if action == "fix" then
    local gap = safe_marker_attr(M, blocking_gap, M._max_blocking_gap_len)
    if gap == "" or not M._is_bounded_string(gap, M._max_blocking_gap_len) then
      error("github-devloop: invalid review-meta gap")
    end
    fields = fields .. '" gap="' .. gap
  elseif action == "spec-amendment" then
    fields = fields .. '" reason="blocked-pending-spec'
  end
  return '<!-- fkst:github-devloop:review-meta:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. fields
    .. '" -->'
end

function M.fix_reflection_marker(issue_proposal_id, dedup_key, verdict, version, fix_round)
  if verdict ~= "checkpoint" and verdict ~= "continue" and verdict ~= "spec-gap" then
    error("github-devloop: invalid fix reflection verdict")
  end
  local n = valid_round(fix_round)
  if n == nil then
    error("github-devloop: invalid fix reflection round")
  end
  local version_field = ""
  if version ~= nil then
    version_field = '" version="' .. tostring(version)
  end
  return '<!-- fkst:github-devloop:fix-reflection:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" verdict="' .. tostring(verdict)
    .. version_field
    .. '" fix_round="' .. tostring(n)
    .. '" -->'
end

function M.fix_marker(issue_proposal_id, review_proposal_id, review_dedup_key, old_head_sha, new_head_sha)
  if not M._is_git_sha(old_head_sha) or not M._is_git_sha(new_head_sha) then
    error("github-devloop: invalid fix head sha")
  end
  return '<!-- fkst:github-devloop:fix:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" review_proposal="' .. tostring(review_proposal_id)
    .. '" review_dedup="' .. tostring(review_dedup_key)
    .. '" old_head_sha="' .. tostring(old_head_sha)
    .. '" new_head_sha="' .. tostring(new_head_sha)
    .. '" -->'
end

function M.merge_gate_marker(issue_proposal_id, pr_number, version, review_proposal_id, review_dedup_key, head_sha, gate_baseline_sha, reason, predecessor_set)
  if not M._is_positive_pr_number(pr_number) or not M._is_git_sha(head_sha) then
    error("github-devloop: invalid merge-gate marker")
  end
  local baseline_field = ""
  if gate_baseline_sha ~= nil then
    if not M._is_git_sha(gate_baseline_sha) then
      error("github-devloop: invalid merge-gate marker")
    end
    baseline_field = '" gate_baseline_sha="' .. tostring(gate_baseline_sha)
  end
  local predecessor_field = ""
  if predecessor_set ~= nil then
    if not M._is_path_safe_key(predecessor_set, M._max_dedup_len) then
      error("github-devloop: invalid merge-gate predecessor set")
    end
    predecessor_field = '" predecessor_set="' .. tostring(predecessor_set)
  end
  return '<!-- fkst:github-devloop:merge-gate:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" review_proposal="' .. tostring(review_proposal_id)
    .. '" review_dedup="' .. tostring(review_dedup_key)
    .. '" head_sha="' .. tostring(head_sha)
    .. baseline_field
    .. predecessor_field
    .. '" reason="' .. tostring(strings.sanitize_key(reason or "gate-failed", false):gsub("/", "-"))
    .. '" -->'
end

function M.implementing_marker(proposal_id, dedup_key, branch, head_sha, base_branch, base_sha)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  if not M._is_git_sha(head_sha) then
    error("github-devloop: invalid head sha")
  end
  if not M._is_git_ref_safe(base_branch) then
    error("github-devloop: invalid base branch")
  end
  if not M._is_git_sha(base_sha) then
    error("github-devloop: invalid base sha")
  end
  return '<!-- fkst:github-devloop:implementing:v1 proposal="' .. tostring(proposal_id)
    .. '" dedup="' .. tostring(dedup_key)
    .. '" branch="' .. tostring(branch)
    .. '" head_sha="' .. tostring(head_sha)
    .. '" base_branch="' .. tostring(base_branch)
    .. '" base_sha="' .. tostring(base_sha)
    .. '" -->'
end

function M.pr_link_marker(proposal_id, pr_number, branch, impl_version, base_branch)
  if not M._is_positive_pr_number(pr_number) then
    error("github-devloop: invalid pr number")
  end
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  if not M._is_git_ref_safe(base_branch) then
    error("github-devloop: invalid base branch")
  end
  return '<!-- fkst:github-devloop:pr-link:v1 proposal="' .. tostring(proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" branch="' .. tostring(branch)
    .. '" impl_version="' .. tostring(impl_version)
    .. '" base_branch="' .. tostring(base_branch)
    .. '" -->'
end

function M.pr_link_marker_template(proposal_id, branch, impl_version, base_branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  if not M._is_git_ref_safe(base_branch) then
    error("github-devloop: invalid base branch")
  end
  return '<!-- fkst:github-devloop:pr-link:v1 proposal="' .. tostring(proposal_id)
    .. '" pr="{{pr_number}}"'
    .. ' branch="' .. tostring(branch)
    .. '" impl_version="' .. tostring(impl_version)
    .. '" base_branch="' .. tostring(base_branch)
    .. '" -->'
end

function M.pr_delegation_marker(issue_proposal_id, pr_proposal_id, pr_number, version, delegation)
  if not M._is_positive_pr_number(pr_number) then
    error("github-devloop: invalid pr-delegation pr number")
  end
  if not M._is_bounded_string(issue_proposal_id, M._max_key_len)
    or not M._is_bounded_string(pr_proposal_id, M._max_key_len)
    or not M._is_bounded_string(version, M._max_dedup_len)
    or not M._is_path_safe_key(delegation, M._max_dedup_len) then
    error("github-devloop: invalid pr-delegation marker")
  end
  return '<!-- fkst:github-devloop:pr-delegation:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr_proposal="' .. tostring(pr_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" delegation="' .. tostring(delegation)
    .. '" -->'
end

function M.pr_terminal_marker(terminal)
  if type(terminal) ~= "table" then
    error("github-devloop: invalid pr-terminal marker")
  end
  local terminal_state = tostring(terminal.terminal or "")
  if terminal_state ~= "merged" and terminal_state ~= "closed-unmerged" and terminal_state ~= "blocked" then
    error("github-devloop: invalid pr-terminal state")
  end
  if not M._is_bounded_string(terminal.pr_proposal, M._max_key_len)
    or select(1, M.parse_pr_proposal_id(terminal.pr_proposal)) == nil
    or not M._is_bounded_string(terminal.repo, M._max_key_len)
    or not M._is_positive_pr_number(terminal.pr_identity)
    or not M._is_positive_pr_number(terminal.pr_number or terminal.pr_identity)
    or tostring(terminal.pr_number or terminal.pr_identity) ~= tostring(terminal.pr_identity)
    or not M._is_path_safe_key(terminal.delegation_generation, M._max_dedup_len)
    or not M._is_git_sha(terminal.head_sha)
    or not M._is_path_safe_key(terminal.terminal_marker_id, M._max_dedup_len) then
    error("github-devloop: invalid pr-terminal marker")
  end
  local marker = '<!-- fkst:github-devloop:pr-terminal:v1 terminal="' .. terminal_state
    .. '" pr_proposal="' .. tostring(terminal.pr_proposal)
    .. '" repo="' .. tostring(terminal.repo)
    .. '" pr_identity="' .. tostring(terminal.pr_identity)
    .. '" pr="' .. tostring(terminal.pr_identity)
    .. '" delegation="' .. tostring(terminal.delegation_generation)
    .. '" head_sha="' .. tostring(terminal.head_sha)
    .. '"'
  if terminal.merge_commit_sha ~= nil then
    if not M._is_git_sha(terminal.merge_commit_sha) then
      error("github-devloop: invalid pr-terminal merge sha")
    end
    marker = marker .. ' merge_commit_sha="' .. tostring(terminal.merge_commit_sha) .. '"'
  end
  return marker
    .. ' terminal_marker_id="' .. tostring(terminal.terminal_marker_id)
    .. '" -->'
end

function M.child_completed_marker(completed)
  if type(completed) ~= "table"
    or not M._is_bounded_string(completed.proposal_id, M._max_key_len)
    or not M._is_bounded_string(completed.pr_proposal, M._max_key_len)
    or not M._is_path_safe_key(completed.pr_source_ref, M._max_key_len)
    or not M._is_path_safe_key(completed.delegation_generation, M._max_dedup_len)
    or not M._is_path_safe_key(completed.terminal_marker_id, M._max_dedup_len)
    or not M._is_path_safe_key(completed.idempotency_key, M._max_dedup_len) then
    error("github-devloop: invalid child-completed marker")
  end
  local terminal_state = tostring(completed.terminal or "")
  if terminal_state ~= "merged" and terminal_state ~= "closed-unmerged" and terminal_state ~= "blocked" then
    error("github-devloop: invalid child-completed terminal")
  end
  return '<!-- fkst:github-devloop:child-completed:v1 proposal="' .. tostring(completed.proposal_id)
    .. '" pr_proposal="' .. tostring(completed.pr_proposal)
    .. '" pr_source_ref="' .. tostring(completed.pr_source_ref)
    .. '" delegation="' .. tostring(completed.delegation_generation)
    .. '" terminal_marker_id="' .. tostring(completed.terminal_marker_id)
    .. '" terminal="' .. terminal_state
    .. '" idempotency_key="' .. tostring(completed.idempotency_key)
    .. '" -->'
end

function M.pr_origin_marker(proposal_id, issue_number, branch, impl_version, base_branch)
  if not M._is_git_ref_safe(branch) then
    error("github-devloop: invalid branch")
  end
  if not M._is_git_ref_safe(base_branch) then
    error("github-devloop: invalid base branch")
  end
  return '<!-- fkst:github-devloop:pr-origin:v1 proposal="' .. tostring(proposal_id)
    .. '" issue="' .. tostring(issue_number)
    .. '" branch="' .. tostring(branch)
    .. '" impl_version="' .. tostring(impl_version)
    .. '" base_branch="' .. tostring(base_branch)
    .. '" -->'
end

function M.review_result_marker(review_proposal_id, issue_proposal_id, decision, dedup_key, fix_round, blocking_gap)
  if decision ~= "approve" and decision ~= "reject" then
    error("github-devloop: invalid review decision")
  end
  local fix_round_field = ""
  local gap_field = ""
  if decision == "reject" then
    if fix_round ~= nil then
      local n = valid_round(fix_round)
      if n == nil then
        error("github-devloop: invalid review reject fix round")
      end
      fix_round_field = '" fix_round="' .. tostring(n)
    end
    local gap = safe_marker_attr(M, blocking_gap, M._max_blocking_gap_len)
    if gap == "" or not M._is_bounded_string(gap, M._max_blocking_gap_len) then
      error("github-devloop: invalid review reject gap")
    end
    gap_field = '" gap="' .. gap
  end
  return '<!-- fkst:github-devloop:review-result:v1 proposal="' .. tostring(review_proposal_id)
    .. '" issue_proposal="' .. tostring(issue_proposal_id)
    .. '" decision="' .. tostring(decision)
    .. '" dedup="' .. tostring(dedup_key)
    .. fix_round_field
    .. gap_field
    .. '" -->'
end

function M.merge_ready_marker(issue_proposal_id, pr_number, version, review_proposal_id, review_dedup_key, head_sha)
  if not M._is_positive_pr_number(pr_number) then
    error("github-devloop: invalid merge-ready pr number")
  end
  if not M._is_git_sha(head_sha) then
    error("github-devloop: invalid merge-ready head sha")
  end
  if not M._is_bounded_string(version, M._max_dedup_len)
    or not M._is_bounded_string(review_proposal_id, M._max_key_len)
    or not M._is_bounded_string(review_dedup_key, M._max_dedup_len) then
    error("github-devloop: invalid merge-ready marker")
  end
  return '<!-- fkst:github-devloop:merge-ready:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" review_proposal="' .. tostring(review_proposal_id)
    .. '" review_dedup="' .. tostring(review_dedup_key)
    .. '" head_sha="' .. tostring(head_sha)
    .. '" -->'
end

function M.review_carry_over_marker(issue_proposal_id, version, old_review_proposal_id, old_review_dedup_key, approved_head_sha, new_review_proposal_id, new_review_dedup_key, new_head_sha, base_head_sha)
  if not M._is_git_sha(approved_head_sha)
    or not M._is_git_sha(new_head_sha)
    or not M._is_git_sha(base_head_sha) then
    error("github-devloop: invalid review carry-over marker")
  end
  if not M._is_bounded_string(version, M._max_dedup_len)
    or not M._is_bounded_string(old_review_proposal_id, M._max_key_len)
    or not M._is_bounded_string(old_review_dedup_key, M._max_dedup_len)
    or not M._is_bounded_string(new_review_proposal_id, M._max_key_len)
    or not M._is_bounded_string(new_review_dedup_key, M._max_dedup_len) then
    error("github-devloop: invalid review carry-over marker")
  end
  return '<!-- fkst:github-devloop:review-carry-over:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" version="' .. tostring(version)
    .. '" old_review_proposal="' .. tostring(old_review_proposal_id)
    .. '" old_review_dedup="' .. tostring(old_review_dedup_key)
    .. '" approved_head_sha="' .. tostring(approved_head_sha)
    .. '" new_review_proposal="' .. tostring(new_review_proposal_id)
    .. '" new_review_dedup="' .. tostring(new_review_dedup_key)
    .. '" new_head_sha="' .. tostring(new_head_sha)
    .. '" base_head_sha="' .. tostring(base_head_sha)
    .. '" proof="merge-tree-empty-delta" -->'
end

function M.merged_marker(issue_proposal_id, pr_number, version, head_sha, autonomy_record)
  if not M._is_positive_pr_number(pr_number) or not M._is_git_sha(head_sha) then
    error("github-devloop: invalid merged marker")
  end
  local autonomy_attrs = autonomy_record ~= nil and (' autonomy_result="v1"' .. M.autonomy_result_marker_attrs(autonomy_record)) or ""
  return '<!-- fkst:github-devloop:merged:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" head_sha="' .. tostring(head_sha) .. '"' .. autonomy_attrs .. ' -->'
end

function M.merging_marker(issue_proposal_id, pr_number, version, head_sha)
  if not M._is_positive_pr_number(pr_number) or not M._is_git_sha(head_sha) then
    error("github-devloop: invalid merging marker")
  end
  return '<!-- fkst:github-devloop:merging:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" version="' .. tostring(version)
    .. '" head_sha="' .. tostring(head_sha)
    .. '" -->'
end

function M.intake_decision_marker(issue_proposal_id, decision, dedup_key, service_class)
  if decision ~= "enable" and decision ~= "track" and decision ~= "decline" and decision ~= "escalate-to-class" then
    error("github-devloop: invalid intake decision")
  end
  if not M._is_bounded_string(dedup_key, M._max_dedup_len) then
    error("github-devloop: invalid intake dedup")
  end
  if not M.is_intake_service_class(service_class) then
    error("github-devloop: invalid intake service class")
  end
  local normalized_class = M.normalize_intake_service_class(service_class)
  return '<!-- fkst:github-devloop:intake-decision:v1 proposal="' .. tostring(issue_proposal_id)
    .. '" decision="' .. tostring(decision)
    .. '" class="' .. normalized_class
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end

function M.orphan_reaped_marker(proposal_id, pr_number, reason)
  if not M._is_positive_pr_number(pr_number) then
    error("github-devloop: invalid orphan reaped pr number")
  end
  local safe_reason = strings.sanitize_key(reason or "parent-terminal", false):gsub("/", "-")
  return '<!-- fkst:github-devloop:orphan-reaped:v1 proposal="' .. tostring(proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" reason="' .. tostring(safe_reason)
    .. '" -->'
end

function M.pr_base_unmanaged_marker(proposal_id, pr_number, pr_base, integration_branch)
  if not M._is_positive_pr_number(pr_number) then
    error("github-devloop: invalid unmanaged-base pr number")
  end
  if not M._is_git_ref_safe(pr_base) or not M._is_git_ref_safe(integration_branch) then
    error("github-devloop: invalid unmanaged-base branch")
  end
  return '<!-- fkst:github-devloop:pr-base-unmanaged:v1 proposal="' .. tostring(proposal_id)
    .. '" pr="' .. tostring(pr_number)
    .. '" reason="pr-base-unmanaged'
    .. '" pr_base="' .. tostring(pr_base)
    .. '" integration_branch="' .. tostring(integration_branch)
    .. '" -->'
end

function M.result_marker(proposal_id, decision, dedup_key)
  if decision ~= "approve" and decision ~= "reject" then
    error("github-devloop: invalid decision")
  end
  return '<!-- fkst:github-devloop:result:v1 proposal="' .. tostring(proposal_id)
    .. '" decision="' .. decision
    .. '" dedup="' .. tostring(dedup_key)
    .. '" -->'
end
end

return S
