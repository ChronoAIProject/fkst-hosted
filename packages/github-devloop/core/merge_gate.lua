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

local function log_check_runs_fallback(M, opts, repo, head_sha, runs, reason)
  if type(M.log_line) ~= "function" then
    return
  end
  M.log_line("info", tostring(opts and opts.dept or "merge"), tostring(opts and opts.proposal_id or "merge-gate"), "CI_FALLBACK", {
    "repo=" .. tostring(repo),
    "head_sha=" .. tostring(head_sha),
    "source=commit-check-runs",
    "required_checks=" .. table.concat(M._required_check_run_names or {}, ","),
    "check_runs=" .. tostring(type(runs) == "table" and #runs or 0),
    "reason=" .. tostring(reason or ""),
  })
end

function M.commit_check_runs_merge_gate(repo, head_sha, opts)
  local result = M.gh_exec({ cmd = M.gh_commit_check_runs_cmd(repo, head_sha), timeout = 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: gh commit check-runs failed: " .. tostring(result.stderr))
  end
  local runs = M.parse_commit_check_runs(result.stdout)
  local green, reason = M.commit_check_runs_green(runs)
  log_check_runs_fallback(M, opts, repo, head_sha, runs, reason)
  return green, reason, runs
end

function M.evaluate_ci_status_gate(pr, opts)
  local green, green_reason = M.pr_rollup_green(pr)
  if not green and green_reason == "missing-status-rollup" and type(opts) == "table" and opts.repo ~= nil then
    local head_sha = tostring(pr and pr.head_sha or "")
    if head_sha ~= "" then
      green, green_reason = M.commit_check_runs_merge_gate(opts.repo, head_sha, opts)
    end
  end
  return green, green_reason
end

function M.evaluate_ci_merge_gate(pr, opts)
  local mergeable, mergeable_reason = M.pr_mergeable(pr)
  if not mergeable then
    return false, mergeable_reason
  end
  local green, green_reason = M.evaluate_ci_status_gate(pr, opts)
  if not green then
    return false, green_reason
  end
  return true, "merge-gate-ok"
end

local merge_gate_reason_classes = {
  ["rollup-red"] = {
    class = "rollup-red",
    requires_pr_merge_product = true,
  },
  ["merge-state-unstable-with-failing-checks"] = {
    class = "rollup-red",
    requires_pr_merge_product = true,
  },
  ["mergeable-conflicting"] = {
    class = "mergeable-conflicting",
    requires_pr_merge_product = false,
  },
}

local function merge_gate_reason_row(reason)
  local text = tostring(reason or "")
  if text:find("^rollup%-red:", 1) ~= nil then
    return merge_gate_reason_classes["rollup-red"]
  end
  return merge_gate_reason_classes[text]
end

function M.merge_gate_reason_class(reason)
  local row = merge_gate_reason_row(reason)
  if row ~= nil then
    return row.class
  end
  local text = tostring(reason or "")
  if M.is_not_mergeable_reason(text) then
    return text
  end
  return M.sanitize_key(text ~= "" and text or "gate-failed", false):gsub("/", "-")
end

function M.merge_gate_reason_requires_pr_merge_product(reason)
  local row = merge_gate_reason_row(reason)
  if row ~= nil then
    return row.requires_pr_merge_product == true
  end
  return M.merge_gate_reason_class(reason) == "rollup-red"
end

function M.ci_missing_status_dispatch_eligible(pr, now_seconds, first_observed_seconds, grace_seconds)
  local green, green_reason = M.pr_rollup_green(pr)
  if green or green_reason ~= "missing-status-rollup" then
    return false, green_reason
  end
  local current_seconds = tonumber(now_seconds)
  local observed_seconds = tonumber(first_observed_seconds)
  local grace = tonumber(grace_seconds or 300)
  if observed_seconds == nil or current_seconds == nil then
    return false, "missing-status-age-unknown"
  end
  local age_seconds = current_seconds - observed_seconds
  if age_seconds < grace then
    return false, "missing-status-grace"
  end
  return true, "missing-status-rollup", age_seconds
end

function M.dispatch_ci_selfheal_once(repo, pr_number, pr, proposal_id, grace_seconds)
  local green, green_reason = M.pr_rollup_green(pr)
  if green or green_reason ~= "missing-status-rollup" then
    return false, green_reason
  end
  local head_sha = tostring(pr and pr.head_sha or "")
  local head_ref = tostring(pr and pr.head_ref_name or "")
  local now_seconds = now()
  local observed_key = M.ci_missing_status_first_observed_key(repo, pr_number, head_sha)
  local first_observed_seconds = tonumber(cache_get(observed_key) or "")
  if first_observed_seconds == nil then
    first_observed_seconds = tonumber(now_seconds)
    if first_observed_seconds == nil then
      return false, "missing-status-age-unknown"
    end
    cache_set(observed_key, tostring(first_observed_seconds))
  end
  local eligible, reason, age_seconds = M.ci_missing_status_dispatch_eligible({
    status_check_rollup = pr and pr.status_check_rollup,
  }, now_seconds, first_observed_seconds, grace_seconds)
  if not eligible then
    return false, reason
  end
  local key = M.ci_dispatch_once_key(repo, pr_number, head_sha)
  local ran = once(key, function()
    local result = M.gh_exec({ cmd = M.gh_workflow_dispatch_ci_cmd(repo, head_ref), timeout = 30 })
    if result.exit_code ~= 0 then
      error("github-devloop: ci workflow dispatch failed: " .. tostring(result.stderr))
    end
    M.log_line("info", "merge", proposal_id, "ci-dispatch-selfheal", {
      "repo=" .. tostring(repo),
      "pr=" .. tostring(pr_number),
      "head_sha=" .. head_sha,
      "head_ref=" .. head_ref,
      "first_observed_seconds=" .. tostring(first_observed_seconds),
      "age_seconds=" .. tostring(age_seconds or ""),
      "once_key=" .. key,
    })
  end)
  if not ran then
    return false, "ci-dispatch-selfheal-already-ran"
  end
  return true, "ci-dispatch-selfheal-dispatched"
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

  local pr_recheck = M.gh_exec({ cmd = M.gh_pr_view_merge_cmd(repo, pr_number), timeout = 30 })
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
  local gate_ok, gate_reason = M.evaluate_ci_merge_gate(rechecked_pr, {
    repo = repo,
    dept = request.dept or "merge",
    proposal_id = request.proposal_id,
  })
  if not gate_ok then
    return false, gate_reason, rechecked_pr
  end
  if type(request.before_merge) == "function" then
    request.before_merge(rechecked_pr)
  end

  local merge_result = M.gh_exec({ cmd = M.gh_pr_merge_cmd(repo, pr_number, request.head_sha), timeout = 120 })
  if merge_result.exit_code ~= 0 then
    error("github-devloop: gh pr merge failed: " .. tostring(merge_result.stderr))
  end

  local merged_view = M.gh_exec({ cmd = M.gh_pr_view_merge_cmd(repo, pr_number), timeout = 30 })
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
