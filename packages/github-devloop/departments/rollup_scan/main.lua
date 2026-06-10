local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_branch_tick" },
  produces = { "devloop_rollup_ready" },
  fanout = { "devloop_branch_tick" },
  stall_window = "5m",
}

local function require_repo(repo)
  local value = tostring(repo or "")
  if value == "" or core.safe_repo(value) ~= value then
    error("github-devloop: FKST_GITHUB_REPO is required for rollup scan")
  end
  return value
end

local function run_cmd(cmd, timeout, error_class)
  local result = exec_sync({ cmd = cmd, timeout = timeout or 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: " .. error_class .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function trim_stdout(result)
  return tostring(result.stdout or ""):gsub("%s+$", "")
end

local function fetch_branch(branch)
  run_cmd(core.git_fetch_branch_cmd("origin", branch), 60, "git rollup fetch")
end

local function remote_head(branch)
  local result = run_cmd(core.git_remote_branch_head_cmd("origin", branch), 30, "git rollup remote head")
  local head = trim_stdout(result)
  if not core.is_safe_head_sha(head) then
    error("github-devloop: unsafe rollup branch head")
  end
  return head
end

local function ahead_count(upstream, integration)
  local result = run_cmd(core.git_ahead_count_cmd(upstream, integration), 30, "git rollup ahead count")
  local text = trim_stdout(result)
  local count = tonumber(text)
  if count == nil or count < 0 then
    error("github-devloop: invalid rollup ahead count")
  end
  return count
end

local function has_content_diff(upstream, integration)
  local result = exec_sync({ cmd = core.git_remote_trees_equal_quiet_cmd(upstream, integration), timeout = 30 })
  if result.exit_code == 0 then
    return false
  end
  if result.exit_code == 1 then
    return true
  end
  error("github-devloop: git rollup content diff failed: " .. tostring(result.stderr))
end

local function list_open_pr(repo, integration, upstream)
  local listed = run_cmd(core.gh_pr_list_head_base_cmd(repo, integration, upstream), 30, "gh rollup PR list")
  local prs = core.parse_pr_list_head_base(listed.stdout)
  if #prs == 0 then
    return nil
  end
  return prs[1]
end

local function create_rollup_pr(repo, upstream, integration, ahead, head_sha)
  local body_file = "/tmp/fkst-github-devloop-rollup-"
    .. core._decimal_checksum(repo .. "#" .. upstream .. "#" .. integration)
    .. ".md"
  local notes = core.draft_release_notes({
    repo = repo,
    upstream_branch = upstream,
    integration_branch = integration,
    head_sha = head_sha,
    ahead = ahead,
    allow_fallback = true,
  })
  file.write(body_file, notes)
  local title = "Roll up " .. tostring(integration) .. " into " .. tostring(upstream)
  run_cmd(core.gh_pr_create_cmd(repo, integration, upstream, title, body_file), 60, "gh rollup PR create")
end

function pipeline(event)
  core.log_entry("rollup_scan", event, "rollup", event and event.queue or "")
  local branches = core.branch_config()
  local cfg = core.devloop_config()
  local repo = require_repo(cfg.repo)

  if branches.integration == branches.upstream then
    core.log_cas_decision("rollup_scan", "rollup", { state = "same-branch", version = branches.upstream }, "tick", "rollup", "skip-idempotent(same-branch)", "integration branch equals upstream branch")
    return
  end

  with_lock(core.rollup_lock_key(repo, branches.upstream, branches.integration), function()
    fetch_branch(branches.upstream)
    fetch_branch(branches.integration)
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
      integration_head = remote_head(branches.integration)
      create_rollup_pr(repo, branches.upstream, branches.integration, ahead, integration_head)
      pr = list_open_pr(repo, branches.integration, branches.upstream)
      if pr == nil then
        error("github-devloop: gh rollup PR create/list did not return an open PR")
      end
    end

    integration_head = integration_head or remote_head(branches.integration)
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

return M
