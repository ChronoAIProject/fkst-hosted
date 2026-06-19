local core = require("core")

local saga = require("std.saga")

local spec = {
  consumes = { "devloop_branch_tick" },
  produces = { "devloop_sync_conflict" },
  fanout = { "devloop_branch_tick" },
  stall_window = "10m",
}

local blocked_by_skew_label = "fkst-dev:blocked-by-skew"

local function require_repo(repo)
  local value = tostring(repo or "")
  if value == "" or core.safe_repo(value) ~= value then
    error("github-devloop: FKST_GITHUB_REPO is required for PR freshness")
  end
  return value
end

local function run_required(result, error_class)
  if result.exit_code ~= 0 then
    error("github-devloop: " .. error_class .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function require_git_ok(result, error_class)
  if result.exit_code ~= 0 then
    error("github-devloop: " .. error_class .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function trim_stdout(result)
  return tostring(result.stdout or ""):gsub("%s+$", "")
end

local function fetch_branch(branch)
  run_required(core.git_fetch_branch("origin", branch, 60), "PR freshness fetch")
end

local function fetch_branches(repo, branches)
  core.with_repo_ref_store_lock(repo, function()
    for _, branch in ipairs(branches) do
      fetch_branch(branch)
    end
  end)
end

local function remote_head(branch)
  local result = run_required(core.git_remote_branch_head("origin", branch, 30), "PR freshness remote head")
  local head = trim_stdout(result)
  if not core.is_safe_head_sha(head) then
    error("github-devloop: unsafe PR freshness branch head")
  end
  return head
end

local function is_ancestor(ancestor_sha, descendant_sha)
  local result = core.git_is_ancestor(ancestor_sha, descendant_sha, 30)
  if result.exit_code == 0 then
    return true
  end
  if result.exit_code == 1 then
    return false
  end
  error("github-devloop: PR freshness ancestor check failed: " .. tostring(result.stderr))
end

local function runtime_root()
  local result = run_required(exec_sync({ cmd = core.read_runtime_root_cmd(), timeout = 30 }), "FKST_RUNTIME_ROOT read")
  return result.stdout
end

local function cleanup_worktree(worktree)
  if worktree == nil then
    return
  end
  local result = core.git_worktree_remove(worktree, 60)
  if result.exit_code ~= 0 then
    core.log_line("warn", "pr_freshness_scan", "pr-freshness", "CLEANUP", {
      "worktree=" .. tostring(worktree),
      "reason=" .. core._one_line(result.stderr or ""),
    })
  end
end

local function with_temp_worktree(runtime, repo, branch, integration, branch_sha, fn)
  local worktree = core.branch_sync_worktree_path(runtime, repo, integration, branch, branch_sha)
  local plan = core.git_worktree_add_detached_plan(worktree, branch_sha)
  run_required(exec_sync({ cmd = core.mkdir_p_cmd(plan.parent_dir), timeout = 30 }), "PR freshness worktree parent directory setup")
  require_git_ok(core.git_worktree_add_detached(plan.worktree, plan.sha, 60), "PR freshness worktree add")

  local ok, result = pcall(fn, worktree)
  cleanup_worktree(worktree)
  if not ok then
    error(result)
  end
  return result
end

local function has_trusted_text(comments, needle)
  if type(comments) ~= "table" then
    return false
  end
  for _, comment in ipairs(core._trusted_marker_comments(comments)) do
    if core._comment_body(comment):find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

local function has_approval_marker(comments, issue_proposal_id, pr_number, head_sha)
  if type(comments) ~= "table" then
    return false
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-result:v1.-%-%->"
  for _, comment in ipairs(core._trusted_marker_comments(comments)) do
    for marker in core._comment_body(comment):gmatch(marker_pattern) do
      local review_proposal = marker:match('proposal="([^"]+)"')
      local _, reviewed_pr_number, _, reviewed_head_sha = core.parse_pr_review_proposal_id(review_proposal)
      if marker:match('decision="([^"]+)"') == "approve"
        and marker:match('issue_proposal="([^"]+)"') == tostring(issue_proposal_id)
        and tostring(reviewed_pr_number or "") == tostring(pr_number or "")
        and tostring(reviewed_head_sha or "") == tostring(head_sha or "") then
        return true
      end
    end
  end
  return false
end

local function issue_state(repo, issue_number)
  if issue_number == nil then
    return { labels = {}, comments = {} }
  end
  local viewed = run_required(core.gh_issue_view_result(repo, issue_number, 30), "PR freshness issue view")
  return core.parse_issue_view_result(viewed.stdout)
end

local function is_blocked_by_skew(pr, issue)
  return core.has_label(issue.labels, blocked_by_skew_label)
    or core.has_label(pr.labels, blocked_by_skew_label)
    or has_trusted_text(issue.comments, "blocked-by-skew")
    or has_trusted_text(pr.comments, "blocked-by-skew")
end

local function is_imminently_mergeable(pr)
  local green, _ = core.evaluate_ci_merge_gate(pr, {})
  return green
end

local function is_approved(pr, origin)
  return has_approval_marker(pr.comments, origin.proposal_id, pr.number, pr.head_sha)
end

local function candidate_reason(pr, origin, issue, state)
  if state.state == "fixing" or state.state == "review-meta" or state.state == "merging" then
    return nil, "arbitrating"
  end
  if is_approved(pr, origin) then
    return "approved"
  end
  if core.merge_ready_fact(pr.comments, origin.proposal_id, state.version, pr.number) ~= nil then
    return "approved"
  end
  if is_blocked_by_skew(pr, issue) and is_imminently_mergeable(pr) then
    return "blocked-by-skew"
  end
  return nil, "not-candidate"
end

local function load_current_pr(repo, pr_number)
  local viewed = run_required(core.gh_pr_view_freshness(repo, pr_number, 30), "PR freshness view")
  return core.parse_pr_view_merge(viewed.stdout)
end

local function list_open_prs(repo)
  local listed = run_required(core.gh_pr_list_freshness(repo, 30), "PR freshness list")
  return core.parse_pr_list_freshness(listed.stdout)
end

local function raise_conflict(repo, branch, integration, branch_sha, integration_sha, pr_number)
  local payload = {
    schema = "github-devloop.v1",
    repo = repo,
    upstream_branch = integration,
    integration_branch = branch,
    upstream_sha = integration_sha,
    integration_sha = branch_sha,
    dedup_key = core.pr_freshness_dedup_key(repo, branch, integration_sha),
    source_ref = core.pr_freshness_source_ref(repo, pr_number),
  }
  core.log_raise("pr_freshness_scan", "pr-freshness", "devloop_sync_conflict", payload)
end

local function write_refresh_commit(worktree, runtime, repo, branch, integration, branch_sha, integration_sha)
  local message_file = core.pr_freshness_message_file(runtime, repo, branch, integration, branch_sha, integration_sha)
  file.write(message_file, core.pr_freshness_commit_message(repo, branch, integration, branch_sha, integration_sha))
  require_git_ok(core.git_commit_message_file(worktree, message_file, 60), "PR freshness commit")
end

local function push_if_real(repo, branch, branch_sha, worktree)
  if core.write_mode() ~= "real" then
    core.log_line("info", "pr_freshness_scan", "pr-freshness", "OUTBOUND", {
      "mode=dry-run",
      "repo=" .. tostring(repo),
      "branch=" .. tostring(branch),
      "branch_sha=" .. tostring(branch_sha),
      "reason=PR freshness push requires FKST_GITHUB_WRITE=1",
    })
    return
  end

  core.assert_trusted_bot_configured()
  fetch_branches(repo, { branch })
  local rechecked_branch_sha = remote_head(branch)
  if rechecked_branch_sha ~= branch_sha then
    core.log_cas_decision("pr_freshness_scan", "pr-freshness", {
      state = "branch",
      version = rechecked_branch_sha,
    }, "freshness", "push", "skip-foreign(head)", "PR branch head changed before push")
    return
  end
  local merge_head = trim_stdout(run_required(core.git_head_sha(worktree, 30), "PR freshness head"))
  if not core.is_safe_head_sha(merge_head) then
    error("github-devloop: unsafe PR freshness merge head")
  end
  require_git_ok(core.git_push_worktree_branch_update_with_lease(worktree, branch, branch_sha, 120), "PR freshness push")
  fetch_branches(repo, { branch })
  local pushed_head = remote_head(branch)
  if pushed_head ~= merge_head then
    error("github-devloop: PR freshness push verification failed")
  end
  core.log_apply("pr_freshness_scan", "pr-freshness", "refreshed", merge_head, {}, {})
end

local function in_managed_scope(repo, branches, pr, origin)
  return tostring(pr.state or ""):upper() == "OPEN"
    and not pr.is_draft
    and origin ~= nil
    and origin.repo == repo
    and origin.branch == pr.head_ref_name
    and origin.base_branch == branches.integration
    and pr.base_ref_name == branches.integration
    and core.is_devloop_issue_branch(pr.head_ref_name)
    and core.is_same_repo_pr_head(pr, repo)
end

local function process_pr(repo, branches, listed_pr)
  local pr = load_current_pr(repo, listed_pr.number)
  pr.number = listed_pr.number
  local origin = core.pr_origin_fact(pr.comments)
  if not in_managed_scope(repo, branches, pr, origin) then
    core.log_cas_decision("pr_freshness_scan", "pr-freshness", { state = nil, version = nil }, "tick", "freshness", "skip-foreign(pr-shape)", "PR is outside managed freshness scope")
    return
  end

  local issue = issue_state(repo, origin.issue_number)
  if not core.verify_pr_review_issue_claim("pr_freshness_scan", origin.repo, origin.issue_number, issue, origin.proposal_id) then
    return
  end
  local state = core.current_entity_state(pr.comments, origin.proposal_id)
  local reason, skip_reason = candidate_reason(pr, origin, issue, state)
  if reason == nil then
    core.log_cas_decision("pr_freshness_scan", origin.proposal_id, state, "tick", "freshness", "skip-idempotent(" .. skip_reason .. ")", "PR is not a freshness candidate")
    return
  end

  with_lock(core.pr_freshness_lock_key(repo, pr.head_ref_name), function()
    fetch_branches(repo, { branches.integration, pr.head_ref_name })
    local integration_sha = remote_head(branches.integration)
    local branch_sha = remote_head(pr.head_ref_name)
    if branch_sha ~= pr.head_sha then
      core.log_cas_decision("pr_freshness_scan", origin.proposal_id, state, "tick", "freshness", "skip-stale(head)", "PR head changed after GitHub read")
      return
    end
    if is_ancestor(integration_sha, branch_sha) then
      core.log_cas_decision("pr_freshness_scan", origin.proposal_id, state, "tick", "freshness", "skip-idempotent(integration-ancestor)", "PR branch already contains integration")
      return
    end

    local runtime = runtime_root()
    with_temp_worktree(runtime, repo, pr.head_ref_name, branches.integration, branch_sha, function(worktree)
      local merge_result = core.git_merge_no_ff(worktree, integration_sha, 120)
      if merge_result.exit_code == 0 then
        write_refresh_commit(worktree, runtime, repo, pr.head_ref_name, branches.integration, branch_sha, integration_sha)
        push_if_real(repo, pr.head_ref_name, branch_sha, worktree)
        return
      end
      local unmerged = core.git_unmerged_paths(worktree, 30)
      if unmerged.exit_code ~= 0 then
        error("github-devloop: PR freshness unmerged path check failed: " .. tostring(unmerged.stderr))
      end
      if tostring(unmerged.stdout or "") ~= "" then
        raise_conflict(repo, pr.head_ref_name, branches.integration, branch_sha, integration_sha, listed_pr.number)
        return
      end
      error("github-devloop: PR freshness merge failed without conflicts: " .. tostring(merge_result.stderr))
    end)
  end)
end

return saga.department(spec, { done = function() return false end, act = function(event)
  core.log_entry("pr_freshness_scan", event, "pr-freshness", event and event.queue or "")
  local branches = core.branch_config()
  local cfg = core.devloop_config()
  local repo = require_repo(cfg.repo)
  if branches.integration == branches.upstream then
    core.log_cas_decision("pr_freshness_scan", "pr-freshness", { state = "same-branch", version = branches.integration }, "tick", "freshness", "skip-idempotent(same-branch)", "integration branch equals upstream branch")
    return
  end
  for _, pr in ipairs(list_open_prs(repo)) do
    process_pr(repo, branches, pr)
  end
end, name = "pr_freshness_scan" })
