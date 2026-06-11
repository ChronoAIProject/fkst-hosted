local S = {}

function S.install(M)
local dashboard_title = "fkst-dev board"
local dashboard_label = "fkst-dashboard"
local dashboard_marker_prefix = "<!-- fkst:dashboard:v1"

local canonical_labels = {
  { name = "fkst-dev:enabled", color = "1D76DB", description = "intake-approved-for-autonomous-development" },
  { name = "fkst-dev:thinking", color = "8250DF", description = "consensus-deliberation-in-progress" },
  { name = "fkst-dev:ready", color = "0E8A16", description = "approved-and-ready-for-implementation" },
  { name = "fkst-dev:implementing", color = "FBCA04", description = "implementation-in-progress" },
  { name = "fkst-dev:pr-open", color = "C5DEF5", description = "implementation-pr-opened" },
  { name = "fkst-dev:reviewing", color = "5319E7", description = "pr-review-consensus-in-progress" },
  { name = "fkst-dev:merge-ready", color = "0E8A16", description = "review-approved-and-ready-to-merge" },
  { name = "fkst-dev:merging", color = "006B75", description = "merge-in-progress" },
  { name = "fkst-dev:merged", color = "0E8A16", description = "implementation-merged" },
  { name = "fkst-dev:fixing", color = "D93F0B", description = "review-rejected-and-fix-in-progress" },
  { name = "fkst-dev:review-meta", color = "BFDADC", description = "review-meta-decision-required" },
  { name = "fkst-dev:impl-failed", color = "B60205", description = "implementation-failed-terminal" },
  { name = "fkst-dev:blocked", color = "B60205", description = "devloop-blocked-terminal" },
  { name = "fkst-dev:blocked-on-dependency", color = "D4C5F9", description = "waiting-for-native-github-issue-dependencies" },
}

local function require_repo(repo)
  local value = tostring(repo or "")
  if value == "" or M.safe_repo(value) ~= value then
    error("github-devloop: FKST_GITHUB_REPO is required for ensure_repo")
  end
  return value
end

local function json_string(value)
  local text = tostring(value or "")
  text = text:gsub("\\", "\\\\")
  text = text:gsub('"', '\\"')
  text = text:gsub("\b", "\\b")
  text = text:gsub("\f", "\\f")
  text = text:gsub("\n", "\\n")
  text = text:gsub("\r", "\\r")
  text = text:gsub("\t", "\\t")
  text = text:gsub("[%z\1-\31]", function(char)
    return string.format("\\u%04x", char:byte())
  end)
  return '"' .. text .. '"'
end

local function run_gh(cmd, timeout, error_class)
  local result = M.gh_exec({ cmd = cmd, timeout = timeout or 30 })
  if result.exit_code ~= 0 then
    error("github-devloop: " .. tostring(error_class) .. " failed: " .. tostring(result.stderr))
  end
  return result
end

local function run_exec(cmd, timeout, error_class)
  local result = exec_sync({ cmd = cmd, timeout = timeout or 30 })
  if result.exit_code ~= 0 then
    return nil, tostring(error_class) .. " failed: " .. tostring(result.stderr)
  end
  return result, nil
end

local function label_index(labels)
  local index = {}
  for _, label in ipairs(labels or {}) do
    if type(label) == "table" and label.name ~= nil then
      index[tostring(label.name)] = label
    end
  end
  return index
end

local function normalize_color(color)
  return tostring(color or ""):upper()
end

local function label_drift(existing, desired)
  return normalize_color(existing.color) ~= normalize_color(desired.color)
    or tostring(existing.description or "") ~= tostring(desired.description or "")
end

local function log_ensure(item, action, fields)
  local parts = {
    "item=" .. tostring(item),
    "action=" .. tostring(action),
  }
  for _, field in ipairs(fields or {}) do
    table.insert(parts, tostring(field))
  end
  M.log_line("info", "ensure_repo", "repo-management-plane", "ENSURE", parts)
end

local function dashboard_anchor_body()
  return "# " .. dashboard_title .. "\n\n"
    .. "This issue is the fkst-dev dashboard anchor. The observability pipeline refreshes this body from trusted markers.\n\n"
    .. M.dashboard_marker("anchor", "1970-01-01T00:00:00Z") .. "\n"
end

local function write_dashboard_anchor_input(repo)
  local path = "/tmp/fkst-github-devloop-dashboard-anchor-" .. M.safe_repo(repo):gsub("/", "-") .. ".json"
  file.write(path, "{"
    .. '"title":' .. json_string(dashboard_title)
    .. ',"body":' .. json_string(dashboard_anchor_body())
    .. ',"labels":[' .. json_string(dashboard_label) .. "]"
    .. "}\n")
  return path
end

local function ensure_labels(repo, mode, existing_labels)
  local existing = label_index(existing_labels)
  local missing = {}
  local drifted = {}
  for _, desired in ipairs(canonical_labels) do
    local current = existing[desired.name]
    if current == nil then
      table.insert(missing, desired)
    elseif label_drift(current, desired) then
      table.insert(drifted, desired)
    else
      log_ensure("label", "unchanged", {
        "repo=" .. repo,
        "name=" .. desired.name,
      })
    end
  end

  if #missing == 0 and #drifted == 0 then
    return { missing = 0, drifted = 0, created = 0, updated = 0 }
  end
  if mode ~= "real" then
    for _, desired in ipairs(missing) do
      log_ensure("label", "create-planned", {
        "mode=" .. tostring(mode),
        "repo=" .. repo,
        "name=" .. desired.name,
      })
    end
    for _, desired in ipairs(drifted) do
      log_ensure("label", "update-planned", {
        "mode=" .. tostring(mode),
        "repo=" .. repo,
        "name=" .. desired.name,
      })
    end
    return { missing = #missing, drifted = #drifted, created = 0, updated = 0 }
  end

  local created = 0
  for _, desired in ipairs(missing) do
    run_gh(M.gh_repo_label_create_cmd(repo, desired.name, desired.color, desired.description), 30, "gh label create")
    created = created + 1
    log_ensure("label", "created", {
      "mode=real",
      "repo=" .. repo,
      "name=" .. desired.name,
    })
  end
  local updated = 0
  for _, desired in ipairs(drifted) do
    run_gh(M.gh_repo_label_update_cmd(repo, desired.name, desired.color, desired.description), 30, "gh label update")
    updated = updated + 1
    log_ensure("label", "updated", {
      "mode=real",
      "repo=" .. repo,
      "name=" .. desired.name,
    })
  end
  return { missing = #missing, drifted = #drifted, created = created, updated = updated }
end

local function ensure_label(repo, mode, existing_labels, desired)
  local current = label_index(existing_labels)[desired.name]
  if current ~= nil and not label_drift(current, desired) then
    log_ensure("label", "unchanged", {
      "repo=" .. repo,
      "name=" .. desired.name,
    })
    return { missing = 0, drifted = 0, created = 0, updated = 0 }
  end
  if mode ~= "real" then
    log_ensure("label", current == nil and "create-planned" or "update-planned", {
      "mode=" .. tostring(mode),
      "repo=" .. repo,
      "name=" .. desired.name,
    })
    return {
      missing = current == nil and 1 or 0,
      drifted = current == nil and 0 or 1,
      created = 0,
      updated = 0,
    }
  end
  if current ~= nil then
    run_gh(M.gh_repo_label_update_cmd(repo, desired.name, desired.color, desired.description), 30, "gh label update")
    log_ensure("label", "updated", {
      "mode=real",
      "repo=" .. repo,
      "name=" .. desired.name,
    })
    return { missing = 0, drifted = 1, created = 0, updated = 1 }
  end
  run_gh(M.gh_repo_label_create_cmd(repo, desired.name, desired.color, desired.description), 30, "gh label create")
  log_ensure("label", "created", {
    "mode=real",
    "repo=" .. repo,
    "name=" .. desired.name,
  })
  return { missing = 1, drifted = 0, created = 1, updated = 0 }
end

local function ensure_dashboard_anchor(repo, mode, issues, bot_login)
  for _, issue in ipairs(issues or {}) do
    if tostring(issue.author_login or "") == tostring(bot_login or "")
      and tostring(issue.body or ""):find(dashboard_marker_prefix, 1, true) ~= nil then
      log_ensure("dashboard-anchor", "unchanged", {
        "repo=" .. repo,
        "issue=" .. tostring(issue.number),
      })
      return { present = true, created = false }
    end
  end

  if mode ~= "real" then
    log_ensure("dashboard-anchor", "create-planned", {
      "mode=" .. tostring(mode),
      "repo=" .. repo,
    })
    return { present = false, created = false }
  end

  local path = write_dashboard_anchor_input(repo)
  run_gh(M.gh_dashboard_issue_create_cmd(repo, path), 30, "gh dashboard anchor create")
  log_ensure("dashboard-anchor", "created", {
    "mode=real",
    "repo=" .. repo,
  })
  return { present = false, created = true }
end

local function ensure_topology(branches)
  if branches.integration == branches.upstream then
    log_ensure("topology", "unchanged", {
      "integration=" .. branches.integration,
      "reason=same-branch",
    })
    return { ok = true, held = false }
  end

  local fetched, fetch_error = run_exec(M.git_fetch_branch_cmd("origin", branches.integration), 60, "git integration branch fetch")
  if fetched == nil then
    log_ensure("topology", "hold", {
      "integration=" .. branches.integration,
      "reason=missing-integration-branch",
      "detail=" .. M._one_line(fetch_error),
    })
    return { ok = false, held = true, reason = "missing-integration-branch" }
  end

  local head, head_error = run_exec(M.git_remote_branch_head_cmd("origin", branches.integration), 30, "git integration branch head")
  if head == nil then
    log_ensure("topology", "hold", {
      "integration=" .. branches.integration,
      "reason=missing-integration-branch",
      "detail=" .. M._one_line(head_error),
    })
    return { ok = false, held = true, reason = "missing-integration-branch" }
  end

  log_ensure("topology", "unchanged", {
    "integration=" .. branches.integration,
    "head=" .. M._trim(head.stdout),
  })
  return { ok = true, held = false, head = M._trim(head.stdout) }
end

function M.dashboard_label()
  return dashboard_label
end

function M.dashboard_marker(hash, generated_at)
  return dashboard_marker_prefix
    .. ' version="' .. tostring(generated_at or "")
    .. '" hash="' .. tostring(hash or "")
    .. '" generated_at="' .. tostring(generated_at or "")
    .. '" -->'
end

function M.dashboard_marker_prefix()
  return dashboard_marker_prefix
end

function M.ensure_repo_label_specs()
  local copy = {}
  for _, label in ipairs(canonical_labels) do
    table.insert(copy, {
      name = label.name,
      color = label.color,
      description = label.description,
    })
  end
  return copy
end

function M.ensure_repo()
  local cfg = M.devloop_config()
  local repo = require_repo(cfg.repo)
  if cfg.write_mode == "real" then
    M.assert_trusted_bot_configured()
  end
  local labels = M.parse_repo_labels(run_gh(M.gh_repo_labels_list_cmd(repo), 30, "gh label list").stdout)
  local dashboard_issues = M.parse_dashboard_issue_list(
    run_gh(M.gh_dashboard_issue_list_cmd(repo, dashboard_label), 30, "gh dashboard issue list").stdout
  )
  local topology_result = ensure_topology({
    upstream = cfg.upstream_branch,
    integration = cfg.integration_branch,
  })
  local apply_mode = cfg.write_mode
  if topology_result.held then
    apply_mode = "held"
  end
  local label_result = ensure_labels(repo, apply_mode, labels)
  local dashboard_label_result = ensure_label(repo, apply_mode, labels, {
    name = dashboard_label,
    color = "ededed",
    description = "fkst observability dashboard singleton",
  })
  local dashboard_result = ensure_dashboard_anchor(repo, apply_mode, dashboard_issues, cfg.bot_login)
  return {
    repo = repo,
    mode = cfg.write_mode,
    labels = label_result,
    dashboard_label = dashboard_label_result,
    dashboard_anchor = dashboard_result,
    topology = topology_result,
  }
end
end

return S
