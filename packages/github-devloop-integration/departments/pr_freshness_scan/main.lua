local git_mechanics = require("devloop.git_mechanics")
local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local error_facts = require("contract.error_facts")
local m_claims = require("devloop.claims")
local pr_safety = require("devloop.pr_safety")
local parsers_misc = require("devloop.parsers.misc")
local parsers_pr = require("devloop.parsers.pr")
local parsers_issue = require("devloop.parsers.issue")
local core = require("core")
local git_adapter = require("forge.git")
local config = require("devloop.config")

local saga = require("workflow.saga")
local m_facts = require("devloop.markers.facts")

local spec = {
  consumes = { "devloop_branch_tick" },
  produces = { "devloop_sync_conflict" },
  fanout = { "devloop_branch_tick" },
  stall_window = "10m",
}

local git = git_adapter.production_handle

local blocked_by_skew_label = "fkst-dev:blocked-by-skew"

local function require_repo(repo)
  local value = tostring(repo or "")
  if value == "" or base_ids.safe_repo(value) ~= value then
    error("github-devloop: FKST_GITHUB_REPO is required for PR freshness")
  end
  return value
end

local function trim_stdout(result)
  return tostring(result.stdout or ""):gsub("%s+$", "")
end

local function cleanup_worktree(worktree)
  if worktree == nil then
    return
  end
  local result = core.git.worktree_remove(worktree, 60)
  if result.exit_code ~= 0 then
    core.log_line("warn", "pr_freshness_scan", "pr-freshness", "CLEANUP", {
      "worktree=" .. tostring(worktree),
      "reason=" .. error_facts.one_line(result.stderr or ""),
    })
  end
end

local function with_temp_worktree(runtime, repo, branch, integration, branch_sha, fn)
  local worktree = core.branch_sync_worktree_path(runtime, repo, integration, branch, branch_sha)
  local plan = git("github-devloop").git_worktree_add_detached_plan(worktree, branch_sha)
  git_mechanics.run_required(exec_sync({ cmd = core.mkdir_p_cmd(plan.parent_dir), timeout = 30 }), "PR freshness worktree parent directory setup")
  git_mechanics.run_required(git("github-devloop").git_worktree_add_detached(plan.worktree, plan.sha, 60), "PR freshness worktree add")

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
  for _, comment in ipairs(parsers_misc._trusted_marker_comments(core, comments)) do
    if parsers_misc._comment_body(core, comment):find(needle, 1, true) ~= nil then
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
  for _, comment in ipairs(parsers_misc._trusted_marker_comments(core, comments)) do
    for marker in parsers_misc._comment_body(core, comment):gmatch(marker_pattern) do
      local review_proposal = marker:match('proposal="([^"]+)"')
      local _, reviewed_pr_number, _, reviewed_head_sha = devloop_base.parse_pr_review_proposal_id(review_proposal)
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
  local viewed = git_mechanics.run_required(core.gh_issue_view_result(repo, issue_number, 30), "PR freshness issue view")
  return parsers_issue.parse_issue_view_result(core, viewed.stdout)
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
  if m_facts.merge_ready_fact(core, pr.comments, origin.proposal_id, state.version, pr.number) ~= nil then
    return "approved"
  end
  if is_blocked_by_skew(pr, issue) and is_imminently_mergeable(pr) then
    return "blocked-by-skew"
  end
  return nil, "not-candidate"
end

local function load_current_pr(repo, pr_number)
  local viewed = git_mechanics.run_required(core.gh_pr_view_freshness(repo, pr_number, 30), "PR freshness view")
  return parsers_pr.parse_pr_view_merge(core, viewed.stdout)
end

local function list_open_prs(repo)
  local listed = git_mechanics.run_required(core.gh_pr_list_freshness(repo, 30), "PR freshness list")
  return parsers_pr.parse_pr_list_freshness(core, listed.stdout)
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
  git_mechanics.run_required(core.git.commit_message_file(worktree, message_file, 60), "PR freshness commit")
end

local function push_if_real(repo, branch, branch_sha, worktree)
  if config.write_mode(core) ~= "real" then
    core.log_line("info", "pr_freshness_scan", "pr-freshness", "OUTBOUND", {
      "mode=dry-run",
      "repo=" .. tostring(repo),
      "branch=" .. tostring(branch),
      "branch_sha=" .. tostring(branch_sha),
      "reason=PR freshness push requires FKST_GITHUB_WRITE=1",
    })
    return
  end

  devloop_base.assert_trusted_bot_configured()
  git_mechanics.fetch_branches(core.git, repo, { branch }, "PR freshness fetch")
  local rechecked_branch_sha = git_mechanics.remote_head(core.git, branch, "PR freshness remote head", "unsafe PR freshness branch head")
  if rechecked_branch_sha ~= branch_sha then
    core.log_cas_decision("pr_freshness_scan", "pr-freshness", {
      state = "branch",
      version = rechecked_branch_sha,
    }, "freshness", "push", "skip-foreign(head)", "PR branch head changed before push")
    return
  end
  local merge_head = trim_stdout(git_mechanics.run_required(git("github-devloop").git_head_sha(worktree, 30), "PR freshness head"))
  if not require("devloop.pr_safety").is_safe_head_sha(merge_head) then
    error("github-devloop: unsafe PR freshness merge head")
  end
  git_mechanics.run_required(git("github-devloop").git_push_worktree_branch_update_with_lease(worktree, branch, branch_sha, 120), "PR freshness push")
  git_mechanics.fetch_branches(core.git, repo, { branch }, "PR freshness fetch")
  local pushed_head = git_mechanics.remote_head(core.git, branch, "PR freshness remote head", "unsafe PR freshness branch head")
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
    and require("devloop.pr_safety").is_devloop_issue_branch(pr.head_ref_name)
    and require("forge.merge.shared").is_same_repo_pr_head(pr, repo)
end

local function process_pr(repo, branches, listed_pr)
  local pr = load_current_pr(repo, listed_pr.number)
  pr.number = listed_pr.number
  local origin = m_facts.pr_origin_fact(core, pr.comments)
  if not in_managed_scope(repo, branches, pr, origin) then
    core.log_cas_decision("pr_freshness_scan", "pr-freshness", { state = nil, version = nil }, "tick", "freshness", "skip-foreign(pr-shape)", "PR is outside managed freshness scope")
    return
  end

  local issue = issue_state(repo, origin.issue_number)
  if not m_claims.verify_pr_review_issue_claim(core, "pr_freshness_scan", origin.repo, origin.issue_number, issue, origin.proposal_id) then
    return
  end
  local state = require("devloop.entity").current_entity_state(core, pr.comments, origin.proposal_id)
  local reason, skip_reason = candidate_reason(pr, origin, issue, state)
  if reason == nil then
    core.log_cas_decision("pr_freshness_scan", origin.proposal_id, state, "tick", "freshness", "skip-idempotent(" .. skip_reason .. ")", "PR is not a freshness candidate")
    return
  end

  with_lock(core.pr_freshness_lock_key(repo, pr.head_ref_name), function()
    git_mechanics.fetch_branches(core.git, repo, { branches.integration, pr.head_ref_name }, "PR freshness fetch")
    local integration_sha = git_mechanics.remote_head(core.git, branches.integration, "PR freshness remote head", "unsafe PR freshness branch head")
    local branch_sha = git_mechanics.remote_head(core.git, pr.head_ref_name, "PR freshness remote head", "unsafe PR freshness branch head")
    if branch_sha ~= pr.head_sha then
      core.log_cas_decision("pr_freshness_scan", origin.proposal_id, state, "tick", "freshness", "skip-stale(head)", "PR head changed after GitHub read")
      return
    end
    if git_mechanics.is_ancestor(core.git, integration_sha, branch_sha, "PR freshness ancestor check") then
      core.log_cas_decision("pr_freshness_scan", origin.proposal_id, state, "tick", "freshness", "skip-idempotent(integration-ancestor)", "PR branch already contains integration")
      return
    end

    local runtime = git_mechanics.runtime_root_with_exec(exec_sync)
    with_temp_worktree(runtime, repo, pr.head_ref_name, branches.integration, branch_sha, function(worktree)
      local merge_result = git_mechanics.git_merge_no_ff(core.git, worktree, integration_sha, 120)
      if merge_result.exit_code == 0 then
        write_refresh_commit(worktree, runtime, repo, pr.head_ref_name, branches.integration, branch_sha, integration_sha)
        push_if_real(repo, pr.head_ref_name, branch_sha, worktree)
        return
      end
      local unmerged = core.git.unmerged_paths(worktree, 30)
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
  local branches = config.branch_config(core)
  local cfg = config.devloop_config(core)
  local repo = require_repo(cfg.repo)
  if branches.integration == branches.upstream then
    core.log_cas_decision("pr_freshness_scan", "pr-freshness", { state = "same-branch", version = branches.integration }, "tick", "freshness", "skip-idempotent(same-branch)", "integration branch equals upstream branch")
    return
  end
  for _, pr in ipairs(list_open_prs(repo)) do
    process_pr(repo, branches, pr)
  end
end, name = "pr_freshness_scan" })
