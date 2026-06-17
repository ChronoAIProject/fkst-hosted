local S = {}
local registry = require("core.registry")
local strings = require("std.strings")

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

local function fetch_commit_check_runs(repo, head_sha)
  if tostring(repo or "") == "" or not M.is_safe_head_sha(head_sha) then
    return nil, "ci-unknown"
  end
  local result = M.gh_exec({ cmd = M.gh_commit_check_runs_cmd(repo, head_sha), timeout = 30 })
  if result.exit_code ~= 0 then
    return nil, "ci-unknown"
  end
  return M.parse_commit_check_runs(result.stdout), nil
end

local function check_run_id(run)
  local id = type(run) == "table" and (run.id or run.databaseId or run.database_id) or nil
  local text = tostring(id or "")
  if text ~= "" and text:find("[^0-9]") == nil then
    return text
  end
  return nil
end

local function check_run_head_sha(run)
  if type(run) ~= "table" then
    return nil
  end
  for _, value in ipairs({
    run.head_sha,
    run.headSha,
    run.headSHA,
  }) do
    if M.is_safe_head_sha(value) then
      return tostring(value):lower()
    end
  end
  if type(run.check_suite) == "table" then
    for _, value in ipairs({
      run.check_suite.head_sha,
      run.check_suite.headSha,
    }) do
      if M.is_safe_head_sha(value) then
        return tostring(value):lower()
      end
    end
  end
  if type(run.checkSuite) == "table" then
    for _, value in ipairs({
      run.checkSuite.head_sha,
      run.checkSuite.headSha,
    }) do
      if M.is_safe_head_sha(value) then
        return tostring(value):lower()
      end
    end
  end
  return nil
end

local function check_run_name(run)
  if type(run) ~= "table" then
    return ""
  end
  return tostring(run.name or run.context or run.workflowName or run.workflow_name or "")
end

local function check_run_state(run)
  if type(run) ~= "table" then
    return "", ""
  end
  return tostring(run.state or run.status or ""):upper(), tostring(run.conclusion or ""):upper()
end

local green_required_check_conclusions = {
  SUCCESS = true,
  NEUTRAL = true,
  SKIPPED = true,
}

local function required_head_check_run_status(runs, head_sha)
  if type(runs) ~= "table" or not M.is_safe_head_sha(head_sha) then
    return "unknown"
  end
  local required_names = M._required_check_run_names or {}
  local required = {}
  for _, name in ipairs(required_names) do
    required[tostring(name)] = false
  end
  local expected = tostring(head_sha):lower()
  for _, run in ipairs(runs) do
    local name = check_run_name(run)
    if required[name] ~= nil then
      local run_head = check_run_head_sha(run)
      if run_head == nil or run_head == expected then
        required[name] = true
        local state, conclusion = check_run_state(run)
        if state == "COMPLETED" then
          if not green_required_check_conclusions[conclusion] then
            return "red"
          end
        else
          return "pending"
        end
      end
    end
  end
  for _, name in ipairs(required_names) do
    if required[tostring(name)] ~= true then
      return "unknown"
    end
  end
  return "green"
end

local function ci_classification(kind, reason, extra)
  local result = extra or {}
  result.kind = kind
  result.reason = reason
  result.merge_blocking = kind ~= "OK"
  result.actionable = kind == "OWN_CI_RED"
  return result
end

local function integration_or_external_red(pr, head_sha, runs)
  local gate_sha = M.rollup_failure_gate_sha(pr)
  if gate_sha ~= nil and tostring(gate_sha):lower() ~= tostring(head_sha):lower() then
    return ci_classification("INTEGRATION_RED", "integration-ci-red", { check_runs = runs })
  end
  return ci_classification("EXTERNAL_CI_RED", "external-ci-red", { check_runs = runs })
end

function M.classify_pr_ci_gate(pr, opts)
  local green, reason = M.pr_rollup_green(pr)
  if green then
    return ci_classification("OK", "rollup-green")
  end
  if reason == "rollup-pending" then
    return ci_classification("CHECKS_PENDING", "checks-pending")
  end
  local repo = opts and opts.repo or nil
  local head_sha = tostring(pr and pr.head_sha or "")
  if not M.is_safe_head_sha(head_sha) then
    return ci_classification("CI_UNKNOWN", "ci-unknown")
  end
  if tostring(repo or "") == "" then
    return ci_classification("CI_UNKNOWN", "ci-unknown")
  end
  local runs, fetch_reason = fetch_commit_check_runs(repo, head_sha)
  if runs == nil then
    return ci_classification("CI_UNKNOWN", fetch_reason or "ci-unknown")
  end
  log_check_runs_fallback(M, opts, repo, head_sha, runs, reason)
  local head_status = required_head_check_run_status(runs, head_sha)
  if head_status == "red" then
    return ci_classification("OWN_CI_RED", "own-ci-red", { check_runs = runs })
  end
  if head_status == "pending" then
    return ci_classification("CHECKS_PENDING", "checks-pending", { check_runs = runs })
  end
  if head_status == "unknown" then
    return ci_classification("CI_UNKNOWN", "ci-unknown", { check_runs = runs })
  end
  if reason == "rollup-red" then
    return integration_or_external_red(pr, head_sha, runs)
  end
  return ci_classification("OK", "rollup-green", { check_runs = runs })
end

function M.rerunnable_check_run_ids_for_head(runs, head_sha)
  if type(runs) ~= "table" or not M.is_safe_head_sha(head_sha) then
    return {}
  end
  local ids = {}
  local seen = {}
  local expected = tostring(head_sha):lower()
  for _, run in ipairs(runs) do
    local id = check_run_id(run)
    local run_head = check_run_head_sha(run)
    if id ~= nil
      and (run_head == nil or run_head == expected)
      and not seen[id] then
      table.insert(ids, id)
      seen[id] = true
    end
  end
  return ids
end

function M.evaluate_ci_status_gate(pr, opts)
  local green, green_reason = M.pr_rollup_green(pr)
  local check_runs = nil
  if not green and green_reason == "missing-status-rollup" and type(opts) == "table" and opts.repo ~= nil then
    local head_sha = tostring(pr and pr.head_sha or "")
    if head_sha ~= "" then
      green, green_reason, check_runs = M.commit_check_runs_merge_gate(opts.repo, head_sha, opts)
    end
  end
  return green, green_reason, check_runs
end

function M.evaluate_ci_merge_gate(pr, opts)
  local mergeable, mergeable_reason = M.pr_mergeable(pr)
  if not mergeable then
    return false, mergeable_reason
  end
  local green, green_reason = M.evaluate_ci_status_gate(pr, opts)
  if not green then
    if green_reason == "rollup-red" then
      local classification = M.classify_pr_ci_gate(pr, opts)
      return false, classification.reason
    end
    return false, green_reason
  end
  return true, "merge-gate-ok"
end

local merge_gate_reason_classes = registry.load_indexed_map("core.merge_gate.reason_classes.index", "reason")

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
  return strings.sanitize_key(text ~= "" and text or "gate-failed", false):gsub("/", "-")
end

function M.merge_gate_reason_requires_pr_merge_product(reason)
  local row = merge_gate_reason_row(reason)
  if row ~= nil then
    return row.requires_pr_merge_product == true
  end
  return false
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

local function merge_ci_selfheal_worktree(repo, pr_number, head_sha)
  local runtime_result = M.gh_exec({ cmd = M.read_runtime_root_cmd(), timeout = 30 })
  if runtime_result.exit_code ~= 0 then
    error("github-devloop: FKST_RUNTIME_ROOT read failed: " .. tostring(runtime_result.stderr))
  end
  local runtime_root = M._trim(runtime_result.stdout)
  if runtime_root == "" or runtime_root:find("[\r\n]") ~= nil then
    error("github-devloop: invalid FKST_RUNTIME_ROOT")
  end
  return runtime_root:gsub("/+$", "")
    .. "/worktrees/merge-ci-selfheal-"
    .. strings.sanitize_key(tostring(repo), false)
    .. "-"
    .. tostring(pr_number)
    .. "-"
    .. tostring(head_sha):sub(1, 12)
end

local function rerequest_head_check_runs(repo, pr_number, head_sha, runs, proposal_id, first_observed_seconds, age_seconds, key)
  local ids = M.rerunnable_check_run_ids_for_head(runs, head_sha)
  if #ids == 0 then
    return false, "ci-selfheal-no-rerunnable-check-runs"
  end
  for _, id in ipairs(ids) do
    local result = M.gh_exec({ cmd = M.gh_check_run_rerequest_cmd(repo, id), timeout = 30 })
    if result.exit_code ~= 0 then
      error("github-devloop: check-run rerequest failed: " .. tostring(result.stderr))
    end
  end
  M.log_line("info", "merge", proposal_id, "ci-selfheal-rerequest", {
    "repo=" .. tostring(repo),
    "pr=" .. tostring(pr_number),
    "head_sha=" .. tostring(head_sha),
    "check_run_count=" .. tostring(#ids),
    "first_observed_seconds=" .. tostring(first_observed_seconds),
    "age_seconds=" .. tostring(age_seconds or ""),
    "once_key=" .. key,
  })
  return true, "ci-selfheal-rerequested"
end

local function nudge_pr_head(repo, pr_number, pr, proposal_id, first_observed_seconds, age_seconds, key)
  local head_sha = tostring(pr and pr.head_sha or "")
  local head_ref = tostring(pr and pr.head_ref_name or "")
  if not M.is_safe_head_sha(head_sha) then
    return false, "ci-selfheal-invalid-head"
  end
  if not M.is_safe_branch(head_ref) then
    return false, "ci-selfheal-invalid-branch"
  end
  if not M.is_same_repo_pr_head(pr, repo) then
    return false, "ci-selfheal-foreign-head"
  end
  local worktree = merge_ci_selfheal_worktree(repo, pr_number, head_sha)
  local remove_result = M.gh_exec({ cmd = M.git_worktree_remove_if_present_cmd(worktree), timeout = 60 })
  if remove_result.exit_code ~= 0 then
    error("github-devloop: merge CI self-heal worktree cleanup failed: " .. tostring(remove_result.stderr))
  end
  local plan = M.git_worktree_add_detached_plan(worktree, head_sha)
  local mkdir_result = exec_sync({ cmd = M.mkdir_p_cmd(plan.parent_dir), timeout = 30 })
  if mkdir_result.exit_code ~= 0 then
    error("github-devloop: merge CI self-heal worktree parent setup failed: " .. tostring(mkdir_result.stderr))
  end
  local add_result = M.git_worktree_add_detached(plan.worktree, plan.sha, 60)
  if add_result.exit_code ~= 0 then
    error("github-devloop: merge CI self-heal worktree add failed: " .. tostring(add_result.stderr))
  end
  local commit_result = M.gh_exec({
    cmd = M.git_empty_commit_cmd(worktree, "chore: nudge PR CI"),
    timeout = 60,
  })
  if commit_result.exit_code ~= 0 then
    error("github-devloop: merge CI self-heal empty commit failed: " .. tostring(commit_result.stderr))
  end
  local push_result = M.git_push_worktree_branch_update_with_lease(worktree, head_ref, head_sha, 120)
  if push_result.exit_code ~= 0 then
    error("github-devloop: merge CI self-heal push failed: " .. tostring(push_result.stderr))
  end
  local pushed_head = M.gh_exec({ cmd = M.git_head_sha_cmd(worktree), timeout = 30 })
  if pushed_head.exit_code ~= 0 then
    error("github-devloop: merge CI self-heal head read failed: " .. tostring(pushed_head.stderr))
  end
  local new_head_sha = tostring(pushed_head.stdout or ""):gsub("%s+$", "")
  if not M.is_safe_head_sha(new_head_sha) or new_head_sha == head_sha then
    error("github-devloop: merge CI self-heal did not create a fresh head")
  end
  M.invalidate_entity_after_write(repo, "pr", pr_number)
  M.log_line("info", "merge", proposal_id, "ci-selfheal-head-nudge", {
    "repo=" .. tostring(repo),
    "pr=" .. tostring(pr_number),
    "old_head_sha=" .. tostring(head_sha),
    "new_head_sha=" .. tostring(new_head_sha),
    "head_ref=" .. head_ref,
    "first_observed_seconds=" .. tostring(first_observed_seconds),
    "age_seconds=" .. tostring(age_seconds or ""),
    "once_key=" .. key,
  })
  return true, "ci-selfheal-head-nudged"
end

function M.ci_selfheal_once(repo, pr_number, pr, proposal_id, grace_seconds, runs)
  local green, green_reason = M.pr_rollup_green(pr)
  if green or green_reason ~= "missing-status-rollup" then
    return false, green_reason
  end
  local head_sha = tostring(pr and pr.head_sha or "")
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
  local key = M.ci_selfheal_once_key(repo, pr_number, head_sha)
  local ran = once(key, function()
    local rerequested, rerequest_reason = rerequest_head_check_runs(
      repo,
      pr_number,
      head_sha,
      runs,
      proposal_id,
      first_observed_seconds,
      age_seconds,
      key
    )
    if rerequested then
      return
    end
    local nudged, nudge_reason = nudge_pr_head(repo, pr_number, pr, proposal_id, first_observed_seconds, age_seconds, key)
    if not nudged then
      error("github-devloop: CI self-heal failed: " .. tostring(rerequest_reason) .. "; " .. tostring(nudge_reason))
    end
  end)
  if not ran then
    return false, "ci-selfheal-already-ran"
  end
  return true, "ci-selfheal-triggered"
end

function M.is_merged_pr(pr)
  return tostring(pr and pr.state or ""):upper() == "MERGED" and tostring(pr and pr.merged_at or "") ~= ""
end

function M.is_match_head_modified_error(stderr)
  return tostring(stderr or ""):find("Head branch was modified", 1, true) ~= nil
end

local function merge_attempt_limit(request)
  local attempts = tonumber(request and request.match_head_retry_attempts or 1) or 1
  attempts = math.floor(attempts)
  if attempts < 1 then
    return 1
  end
  return attempts
end

local function expected_pr_identity(request, repo, head_sha)
  return {
    repo = repo,
    head_sha = head_sha,
    head_branch = request and request.head_branch,
    base_branch = request and request.base_branch,
  }
end

function M.run_verified_pr_merge(request)
  local repo = tostring(request and request.repo or "")
  local pr_number = request and request.pr_number
  local max_attempts = merge_attempt_limit(request)
  for attempt = 1, max_attempts do
    local pr_recheck = M.gh_exec({ cmd = M.gh_pr_view_merge_cmd(repo, pr_number), timeout = 30 })
    if pr_recheck.exit_code ~= 0 then
      error("github-devloop: gh pr merge recheck failed: " .. tostring(pr_recheck.stderr))
    end
    local rechecked_pr = M.parse_pr_view_merge(pr_recheck.stdout)
    rechecked_pr.number = pr_number
    local merge_head_sha = request and request.head_sha
    if request and request.accept_current_head == true then
      merge_head_sha = rechecked_pr.head_sha
      if not M.is_safe_head_sha(merge_head_sha) then
        return false, "invalid-current-head-sha", rechecked_pr
      end
    end
    local expected = expected_pr_identity(request, repo, merge_head_sha)
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

    local merge_result = M.gh_exec({ cmd = M.gh_pr_merge_cmd(repo, pr_number, merge_head_sha), timeout = 120 })
    if merge_result.exit_code ~= 0 then
      if attempt < max_attempts and M.is_match_head_modified_error(merge_result.stderr) then
        M.log_line("info", tostring(request.dept or "merge"), tostring(request.proposal_id or "merge"), "MATCH_HEAD_RETRY", {
          "repo=" .. tostring(repo),
          "pr=" .. tostring(pr_number),
          "head_sha=" .. tostring(merge_head_sha),
          "attempt=" .. tostring(attempt),
          "max_attempts=" .. tostring(max_attempts),
          "reason=head-branch-modified",
        })
      else
        error("github-devloop: gh pr merge failed: " .. tostring(merge_result.stderr))
      end
    else
      M.invalidate_entity_after_write(repo, "pr", pr_number)

      local merged_view = M.gh_exec({ cmd = M.gh_pr_view_merge_cmd(repo, pr_number), timeout = 30 })
      if merged_view.exit_code ~= 0 then
        error("github-devloop: gh pr post-merge view failed: " .. tostring(merged_view.stderr))
      end
      local merged_pr = M.parse_pr_view_merge(merged_view.stdout)
      merged_pr.number = pr_number
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
  error("github-devloop: gh pr merge failed: Head branch was modified after bounded retry")
end
end

return S
