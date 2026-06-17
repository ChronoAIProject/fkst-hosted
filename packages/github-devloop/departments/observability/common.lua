local M = {}

M.dept = "observability"
M.dashboard_title = "fkst-dev board"
M.dashboard_label = "fkst-dashboard"
M.dashboard_marker_prefix = "<!-- fkst:dashboard:v1"
M.max_dashboard_body_len = 12000
M.max_dashboard_section_items = 40
M.max_dashboard_title_len = 80
M.max_reap_reason_len = 180
M.stall_suspect_threshold_minutes = {
  thinking = 30,
  ready = 30,
  implementing = 90,
  ["pr-open"] = 30,
  reviewing = 60,
  fixing = 90,
  merging = 30,
}

function M.install_common(_core)
end

function M.json_string(value)
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

function M.stderr_http_status(stderr)
  local text = tostring(stderr or "")
  local status = text:match("[Hh][Tt][Tt][Pp][^%d]*(%d%d%d)")
    or text:match("status[^\n%d]*(%d%d%d)")
  return status or "unknown"
end

function M.gh_auth_mode(core)
  if core.env_present("GH_TOKEN") or core.env_present("GITHUB_TOKEN") then
    return "env-token"
  end
  return "gh-auth"
end

function M.command_indicates_not_found(result)
  local stderr = tostring(result and result.stderr or "")
  return stderr:find("404", 1, true) ~= nil
    or stderr:lower():find("not found", 1, true) ~= nil
end

function M.command_indicates_already_exists(result)
  local stderr = tostring(result and result.stderr or ""):lower()
  return stderr:find("already exists", 1, true) ~= nil
    or stderr:find("name already exists", 1, true) ~= nil
    or stderr:find("422", 1, true) ~= nil
    or stderr:find("409", 1, true) ~= nil
end

function M.dashboard_deferred_if_deadline(core, deadline)
  if core.observability_has_budget(deadline) then return nil end
  log.info("github-devloop dept=observability tag=DASHBOARD_DEFERRED reason=deadline")
  return "deferred"
end

function M.require_observe_repo(core)
  local repo = core.read_env("FKST_GITHUB_REPO")
  if repo == nil or core.safe_repo(repo) ~= tostring(repo) then
    error("github-devloop: FKST_GITHUB_REPO is required for observability")
  end
  return repo
end

function M.require_observe_bot(core)
  local login = core.assert_trusted_bot_configured()
  if login == nil or tostring(login) == "" then
    error("github-devloop: FKST_GITHUB_BOT_LOGIN is required for observability")
  end
end

function M.fetch_issue(core, repo, issue_number, limits, deadline)
  local view = core.observability_run_cmd({
    run = function(timeout)
      return core.gh_issue_view_observe(repo, issue_number, timeout)
    end,
  }, limits, deadline, "observability issue view")
  if core.observability_result_deferred(view) then
    return nil
  end
  return core.parse_issue_view_observe(view.stdout)
end

function M.fetch_pr(core, repo, pr_number, limits, deadline)
  local view = core.observability_run_cmd({
    run = function(timeout)
      return core.gh_pr_view_observe(repo, pr_number, timeout)
    end,
  }, limits, deadline, "observability PR view")
  if core.observability_result_deferred(view) then
    return nil
  end
  return core.parse_pr_view_origin(view.stdout)
end

return M
