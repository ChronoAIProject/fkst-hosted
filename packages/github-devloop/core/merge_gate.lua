local S = {}

function S.install(M)
local function is_open_pr(pr)
  return tostring(pr.state or ""):upper() == "OPEN"
end

function M.pr_identity_matches(pr, expected)
  if type(pr) ~= "table" then
    return false, "missing-pr"
  end
  if not is_open_pr(pr) then
    return false, "pr-not-open"
  end
  if tostring(pr.head_sha or "") ~= tostring(expected and expected.head_sha or "") then
    return false, "head-sha-mismatch"
  end
  if tostring(pr.head_ref_name or "") ~= tostring(expected and expected.head_branch or "") then
    return false, "head-branch-mismatch"
  end
  if tostring(pr.base_ref_name or "") ~= tostring(expected and expected.base_branch or "") then
    return false, "base-branch-mismatch"
  end
  if not M.is_same_repo_pr_head(pr, expected and expected.repo) then
    return false, "foreign-head-repository"
  end
  return true, "pr-ok"
end

function M.evaluate_ci_merge_gate(pr)
  local green, green_reason = M.pr_rollup_green(pr)
  if not green then
    return false, green_reason
  end
  local mergeable, mergeable_reason = M.pr_mergeable(pr)
  if not mergeable then
    return false, mergeable_reason
  end
  return true, "merge-gate-ok"
end

function M.is_merged_pr(pr)
  return tostring(pr and pr.state or ""):upper() == "MERGED" and tostring(pr and pr.merged_at or "") ~= ""
end

function M.run_verified_pr_merge(request)
  local repo = tostring(request and request.repo or "")
  local pr_number = request and request.pr_number
  local expected = {
    repo = repo,
    head_sha = request and request.head_sha,
    head_branch = request and request.head_branch,
    base_branch = request and request.base_branch,
  }

  local pr_recheck = exec_sync({ cmd = M.gh_pr_view_merge_cmd(repo, pr_number), timeout = 30 })
  if pr_recheck.exit_code ~= 0 then
    error("github-devloop: gh pr merge recheck failed: " .. tostring(pr_recheck.stderr))
  end
  local rechecked_pr = M.parse_pr_view_merge(pr_recheck.stdout)
  local identity_ok, identity_reason = M.pr_identity_matches(rechecked_pr, expected)
  if not identity_ok then
    return false, identity_reason, rechecked_pr
  end
  if type(request.validate_rechecked_pr) == "function" then
    local validate_ok, validate_reason = request.validate_rechecked_pr(rechecked_pr)
    if not validate_ok then
      return false, validate_reason or "pr-validation-failed", rechecked_pr
    end
  end
  local gate_ok, gate_reason = M.evaluate_ci_merge_gate(rechecked_pr)
  if not gate_ok then
    return false, gate_reason, rechecked_pr
  end
  if type(request.before_merge) == "function" then
    request.before_merge(rechecked_pr)
  end

  local merge_result = exec_sync({ cmd = M.gh_pr_merge_cmd(repo, pr_number, request.head_sha), timeout = 120 })
  if merge_result.exit_code ~= 0 then
    error("github-devloop: gh pr merge failed: " .. tostring(merge_result.stderr))
  end

  local merged_view = exec_sync({ cmd = M.gh_pr_view_merge_cmd(repo, pr_number), timeout = 30 })
  if merged_view.exit_code ~= 0 then
    error("github-devloop: gh pr post-merge view failed: " .. tostring(merged_view.stderr))
  end
  local merged_pr = M.parse_pr_view_merge(merged_view.stdout)
  if not M.is_merged_pr(merged_pr) then
    return false, "merge-confirmation-pending", merged_pr
  end
  if tostring(merged_pr.head_ref_name or "") ~= tostring(expected.head_branch or "")
    or tostring(merged_pr.head_sha or "") ~= tostring(expected.head_sha or "")
    or tostring(merged_pr.base_ref_name or "") ~= tostring(expected.base_branch or "")
    or not M.is_same_repo_pr_head(merged_pr, repo) then
    return false, "merge-confirmation-mismatch", merged_pr
  end
  return true, "merged", merged_pr
end
end

return S
