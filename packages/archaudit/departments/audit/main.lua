local core = require("core")
local codex = require("contract.codex")
local env = require("contract.env")
local saga = require("contract.saga")
local ports_lib = require("std.ports")
local strings = require("contract.strings")

local spec = {
  consumes = { "idle-detector.system_idle", "archaudit_tick" },
  produces = { "github-proxy.github_issue_create_request" },
  stall_window = "10m",
  retry = false,
}

local freshness_budget_seconds = 10 * 60
local codex_timeout_seconds = 9 * 60
local allowed_env = {
  FKST_GITHUB_REPO = true,
  FKST_GITHUB_BOT_LOGIN = true,
  ARCHAUDIT_MAX_ISSUES_PER_IDLE = true,
}

local function read_env_command(name)
  if not allowed_env[name] then
    error("archaudit: invalid-env-name: env name is not allowed")
  end
  return 'printf %s "$' .. name .. '"'
end

local read_env = env.read_env(read_env_command)

local function log_fact(level, dept, tag, error_class, event, message, terminal)
  if tag == "SKIP" then
    log[level or "warn"](core.skip_fact(dept, event, message, terminal))
  else
    log[level or "warn"](core.failure_fact(dept, tag, error_class, event, message, terminal))
  end
end

local function fail(event, error_class, message)
  log_fact("error", "audit", "FAILURE", error_class, event, message, true)
  error(("archaudit: " .. tostring(error_class) .. ": " .. tostring(message)), 0)
end

local function fresh_hint(payload, now_seconds)
  local detected = core.iso_timestamp_epoch_seconds(payload.detected_at)
  if detected == nil then
    error("archaudit: malformed-idle-hint: malformed detected_at")
  end
  local expires = nil
  if payload.expires_at ~= nil then
    expires = core.iso_timestamp_epoch_seconds(payload.expires_at)
    if expires == nil then
      error("archaudit: malformed-idle-hint: malformed expires_at")
    end
  end
  local verdict = core.idle_hint_freshness(detected, expires, now_seconds, freshness_budget_seconds)
  if verdict == "stale" then
    return false, "stale system_idle hint"
  end
  if verdict == "expired" then
    return false, "expired system_idle hint"
  end
  return true, nil
end

local function max_issues()
  local raw = strings.trim(read_env("ARCHAUDIT_MAX_ISSUES_PER_IDLE") or "")
  local value = tonumber(raw)
  if value == nil or value < 1 then
    return 3
  end
  return math.floor(value)
end

local function bot_login()
  return strings.trim(read_env("FKST_GITHUB_BOT_LOGIN") or "")
end

local function repo_from_env()
  local repo = strings.trim(read_env("FKST_GITHUB_REPO") or "")
  if repo == "" then
    return nil, "missing-repo", "missing FKST_GITHUB_REPO"
  end
  if not core.validate_repo(repo) then
    return nil, "malformed-repo", "malformed FKST_GITHUB_REPO"
  end
  return repo, nil, nil
end

local function has_archaudit_label(github, repo)
  if type(github) ~= "table" or type(github.label_list) ~= "function" then
    return false
  end
  local ok, result = pcall(function()
    return github.label_list(repo, 30)
  end)
  if not ok or type(result) ~= "table" or result.exit_code ~= 0 then
    return false
  end
  local ok_json, labels = pcall(json.decode, result.stdout or "[]")
  if not ok_json or type(labels) ~= "table" then
    return false
  end
  for _, label in ipairs(labels) do
    if type(label) == "table" and label.name == "archaudit" then
      return true
    end
  end
  return false
end

local function audit_issues(github, repo)
  if type(github) ~= "table" or type(github.issue_search) ~= "function" then
    error("archaudit: audit-search-unavailable: GitHub issue_search port is required")
  end
  local result = github.issue_search(repo, core.audit_issue_search_query(), "number,title,state,author,body,url,createdAt,updatedAt", 30)
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("archaudit: audit-search-failed: GitHub audit issue search failed")
  end
  return core.parse_audit_issue_search(result.stdout)
end

local function parser_error_class(err)
  local text = tostring(err)
  if text:find("malformed-json", 1, true) ~= nil then
    return "malformed-json"
  end
  if text:find("non-array-json", 1, true) ~= nil then
    return "non-array-json"
  end
  return "validation-failure"
end

local function observe_result()
  return pcall(core.observe)
end

local function observe_now_result(facts)
  return pcall(core.observe_now_seconds, facts)
end

local function idle_observe_result(facts)
  return pcall(core.is_idle_observe, facts)
end

local function run_codex(repo, max_count)
  local opts = codex.judgment_codex_opts(core.build_prompt(repo, max_count), ".")
  opts.timeout = codex_timeout_seconds
  local result = spawn_codex_sync(opts)
  if type(result) ~= "table" or result.exit_code ~= 0 then
    local code = type(result) == "table" and tonumber(result.exit_code) or nil
    if code == 124 then
      error("archaudit: codex-timeout: codex timeout")
    end
    error("archaudit: codex-nonzero: codex nonzero exit")
  end
  return core.parse_findings_json(result.stdout)
end

local function codex_result(repo, count)
  return pcall(run_codex, repo, count)
end

local function issue_request_result(repo, finding, label_available, trigger_reason)
  return pcall(core.build_issue_create_request, repo, finding, label_available, trigger_reason)
end

local function audit_run_request_result(repo, trigger_reason, label_available, now_seconds, max_staleness)
  return pcall(core.build_audit_run_issue_create_request, repo, trigger_reason, label_available, now_seconds, max_staleness)
end

local function stop_observe_error(event, err)
  local message = tostring(err)
  if message:find("observe%-unreadable") ~= nil then
    log_fact("warn", "audit", "SKIP", "terminal-skip", event, message, true)
    return true
  end
  fail(event, "observe-malformed", message)
end

local function fail_observe_malformed(event, err)
  fail(event, "observe-malformed", tostring(err))
end

local function fail_codex_error(event, err)
  local message = tostring(err)
  if message:find("codex%-timeout") ~= nil then
    fail(event, "codex-timeout", "codex timeout")
  end
  if message:find("codex%-nonzero") ~= nil then
    fail(event, "codex-nonzero", "codex nonzero exit")
  end
  fail(event, parser_error_class(message), message)
end

local function fail_request_error(event, err)
  fail(event, "validation-failure", err)
end

local function trigger_kind(event)
  local queue = type(event) == "table" and tostring(event.queue or "") or ""
  if queue == "idle-detector.system_idle" then
    return "idle"
  end
  if queue == "archaudit.archaudit_tick" or queue == "archaudit_tick" then
    return "stale"
  end
  return nil
end

local function audit_done(event)
  local trigger = trigger_kind(event)
  if trigger == nil then
    fail(event, "unknown-queue", "unknown queue")
  end
  local payload = event.payload or {}
  if trigger == "idle" and payload.schema ~= "idle-detector.system-idle.v1" then
    fail(event, "unknown-schema", "unknown system_idle schema")
  end
  if trigger == "stale" and not core.validate_audit_tick_payload(payload) then
    fail(event, "unknown-schema", "unknown archaudit_tick schema")
  end
  return false
end

local function make_department(ports)
  local function act_audit(event)
    local payload = event.payload or {}
    local trigger = trigger_kind(event)
    if trigger == nil then
      fail(event, "unknown-queue", "unknown queue")
    end
    local ok_observe, facts_or_err = observe_result()
    if not ok_observe and stop_observe_error(event, facts_or_err) then
      return
    end
    local ok_time, observe_now_or_err = observe_now_result(facts_or_err)
    if not ok_time and fail_observe_malformed(event, observe_now_or_err) then
      return
    end

    if trigger == "idle" then
      local ok_fresh, fresh, fresh_why = pcall(fresh_hint, payload, observe_now_or_err)
      if not ok_fresh then
        fail(event, "malformed-idle-hint", fresh)
      end
      if not fresh then
        log_fact("warn", "audit", "SKIP", "terminal-skip", event, fresh_why, true)
        return
      end
      local ok_idle, idle, why = idle_observe_result(facts_or_err)
      if not ok_idle and fail_observe_malformed(event, idle) then
        return
      end
      if not idle then
        log_fact("warn", "audit", "SKIP", "terminal-skip", event, why or "current system busy", true)
        return
      end
    elseif not core.validate_audit_tick_payload(payload) then
      fail(event, "unknown-schema", "unknown archaudit_tick schema")
    end

    local repo, repo_error_class, repo_error = repo_from_env()
    if repo == nil then
      fail(event, repo_error_class, repo_error)
    end
    local ok_audit_search, issues_or_err = pcall(audit_issues, ports.github, repo)
    if not ok_audit_search then
      fail(event, "audit-search-failed", issues_or_err)
    end
    local staleness_seconds = core.audit_due_staleness_seconds()
    local due, due_why = core.audit_due_verdict(issues_or_err, bot_login(), observe_now_or_err, staleness_seconds)
    if not due then
      log_fact("warn", "audit", "SKIP", "terminal-skip", event, due_why, true)
      return
    end

    local count = max_issues()
    local ok_codex, findings_or_err = codex_result(repo, count)
    if not ok_codex and fail_codex_error(event, findings_or_err) then
      return
    end

    local label_available = has_archaudit_label(ports.github, repo)
    local requests = {}
    if #findings_or_err == 0 then
      local ok_request, request_or_err = audit_run_request_result(repo, trigger, label_available, observe_now_or_err, staleness_seconds)
      if not ok_request and fail_request_error(event, request_or_err) then
        return
      end
      table.insert(requests, request_or_err)
    end
    for _, finding in ipairs(findings_or_err) do
      if #requests >= count then
        break
      end
      if not core.validate_finding(finding) then
        fail(event, "validation-failure", "invalid file or line")
      end
      local ok_request, request_or_err = issue_request_result(repo, finding, label_available, trigger)
      if not ok_request and fail_request_error(event, request_or_err) then
        return
      end
      table.insert(requests, request_or_err)
    end
    for _, request in ipairs(requests) do
      raise("github-proxy.github_issue_create_request", request)
    end
  end

  local previous_pipeline = _G.pipeline
  local department = saga.department(spec, {
    done = audit_done,
    act = act_audit,
    name = "audit",
  })
  department.pipeline = _G.pipeline
  _G.pipeline = previous_pipeline
  department.ports = ports
  return department
end

local M = ports_lib.install(make_department)
_G.pipeline = M.pipeline
return M
