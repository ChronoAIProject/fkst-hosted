local git_mechanics = require("devloop.git_mechanics")
local base_ids = require("devloop.base_ids")
local parsers_pr = require("devloop.parsers.pr")
local core = require("core")
local saga = require("workflow.saga")
local github = require("forge.github").production_handle
local config = require("devloop.config")

local spec = {
  consumes = { "devloop_branch_tick" },
  produces = { "devloop_rollup_ready", "github-proxy.github_issue_create_request" },
  fanout = { "devloop_branch_tick" },
  stall_window = "5m",
}

local function done(_event)
  return false
end

local function require_repo(repo)
  local value = tostring(repo or "")
  if value == "" or base_ids.safe_repo(value) ~= value then
    error("github-devloop: FKST_GITHUB_REPO is required for rollup scan")
  end
  return value
end

local function trim_stdout(result)
  return tostring(result.stdout or ""):gsub("%s+$", "")
end

local function ahead_count(upstream, integration)
  local result = git_mechanics.run_required(core.git_ahead_count(upstream, integration, 30), "rollup ahead count")
  local text = trim_stdout(result)
  local count = tonumber(text)
  if count == nil or count < 0 then
    error("github-devloop: invalid rollup ahead count")
  end
  return count
end

local function has_content_diff(upstream, integration)
  local result = git_mechanics.git_remote_trees_equal_quiet(core.git, upstream, integration, 30)
  if result.exit_code == 0 then
    return false
  end
  if result.exit_code == 1 then
    return true
  end
  error("github-devloop: rollup content diff failed: " .. tostring(result.stderr))
end

local function list_open_pr(repo, integration, upstream)
  local listed = git_mechanics.run_required(core.gh_pr_list_head_base(repo, integration, upstream, 30), "rollup PR list")
  local prs = parsers_pr.parse_pr_list_head_base(core, listed.stdout)
  if #prs == 0 then
    return nil
  end
  return prs[1]
end

local function fetch_rollup_pr(repo, pr_number)
  local viewed = git_mechanics.run_required(github("github-devloop-integration.rollup_scan").gh_pr_view_merge(repo, pr_number, 30), "rollup PR view")
  local pr = parsers_pr.parse_pr_view_merge(core, viewed.stdout)
  pr.number = tonumber(pr_number)
  return pr
end

local function is_no_commits_between_error(stderr, upstream, integration)
  local text = tostring(stderr or "")
  local expected = "No commits between " .. tostring(upstream) .. " and " .. tostring(integration)
  return text:find(expected, 1, true) ~= nil
end

local function create_rollup_pr(repo, upstream, integration, head_sha, ahead, publish_policy)
  local notes = core.draft_release_notes({
    repo = repo,
    upstream_branch = upstream,
    integration_branch = integration,
    head_sha = head_sha,
    ahead = ahead,
    publish_policy = publish_policy,
  })
  local title = "Roll up " .. tostring(integration) .. " into " .. tostring(upstream)
  local result = core.gh_pr_create_body(repo, integration, upstream, title, notes, 60)
  if result.exit_code == 0 then
    return true
  end
  if is_no_commits_between_error(result.stderr, upstream, integration) then
    return false
  end
  error("github-devloop: rollup PR create failed: " .. tostring(result.stderr))
end

local function act(event)
  core.log_entry("rollup_scan", event, "rollup", event and event.queue or "")
  local branches = config.branch_config(core)
  local cfg = config.devloop_config(core)
  local repo = require_repo(cfg.repo)

  if branches.integration == branches.upstream then
    core.log_cas_decision("rollup_scan", "rollup", { state = "same-branch", version = branches.upstream }, "tick", "rollup", "skip-idempotent(same-branch)", "integration branch equals upstream branch")
    return
  end

  with_lock(core.rollup_lock_key(repo, branches.upstream, branches.integration), function()
    git_mechanics.fetch_branches(core.git, repo, { branches.upstream, branches.integration }, "rollup fetch")
    local ahead = ahead_count(branches.upstream, branches.integration)
    if ahead == 0 then
      core.log_cas_decision("rollup_scan", "rollup", { state = "not-ahead", version = branches.integration }, "tick", "rollup", "skip-idempotent(not-ahead)", "integration is not ahead of upstream")
      return
    end
    if not has_content_diff(branches.upstream, branches.integration) then
      core.log_cas_decision("rollup_scan", "rollup", { state = "empty-diff", version = branches.integration }, "tick", "rollup", "skip-idempotent(empty-diff)", "integration has no content diff from upstream")
      return
    end

    local integration_head = nil
    local pr = list_open_pr(repo, branches.integration, branches.upstream)
    if pr == nil then
      if cfg.write_mode ~= "real" then
        core.log_line("info", "rollup_scan", "rollup", "OUTBOUND", {
          "mode=dry-run",
          "repo=" .. repo,
          "upstream=" .. branches.upstream,
          "integration=" .. branches.integration,
          "reason=rollup PR create requires FKST_GITHUB_WRITE=1",
        })
        return
      end
      integration_head = git_mechanics.remote_head(core.git, branches.integration, "rollup remote head", "unsafe rollup branch head")
      local created = create_rollup_pr(
        repo,
        branches.upstream,
        branches.integration,
        integration_head,
        ahead,
        core.release_notes_publish_policy(cfg)
      )
      if not created then
        core.log_cas_decision("rollup_scan", "rollup", { state = "not-ahead", version = branches.integration }, "tick", "rollup", "skip-idempotent(no-commits-between)", "GitHub reports no commits between upstream and integration")
        return
      end
      pr = list_open_pr(repo, branches.integration, branches.upstream)
      if pr == nil then
        error("github-devloop: rollup PR create/list did not return an open PR")
      end
    end

    integration_head = integration_head or git_mechanics.remote_head(core.git, branches.integration, "rollup remote head", "unsafe rollup branch head")
    core.observe_rollup_health(
      repo,
      branches.upstream,
      branches.integration,
      fetch_rollup_pr(repo, pr.number),
      now(),
      core.rollup_red_window_minutes()
    )
    if cfg.rollup_merge == "manual" then
      core.log_line("info", "rollup_scan", "rollup", "POSTURE", {
        "posture=manual",
        "repo=" .. repo,
        "pr=" .. tostring(pr.number),
        "reason=open/update only, no merge event",
      })
      return
    end

    local payload = core.rollup_ready_payload(repo, branches.upstream, branches.integration, pr.number, integration_head)
    core.log_raise("rollup_scan", "rollup", "devloop_rollup_ready", payload)
  end)
end

return saga.department(spec, {
  done = done,
  act = act,
  wrap = core.wrap_pipeline_failure,
  name = "rollup_scan",
})
