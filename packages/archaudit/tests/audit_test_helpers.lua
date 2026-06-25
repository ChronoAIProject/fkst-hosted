local testing = require("testkit.testing")
local github_fake = require("forge.github_fake")
local audit_main = require("departments.audit.main")
local t = fkst.test

local function run_department_opts()
  return {
    env = {
      FKST_GITHUB_REPO = "owner/repo",
      FKST_GITHUB_BOT_LOGIN = "fkst-test-bot",
      ARCHAUDIT_MAX_ISSUES_PER_IDLE = "3",
    },
  }
end

local function idle_event(extra)
  local detected_at = "1970-01-01T00:00:00Z"
  local payload = {
    schema = "idle-detector.system-idle.v1",
    detected_at = detected_at,
    expires_at = "1970-01-01T00:10:00Z",
    source_ref = { kind = "host-observe", ref = "idle_tick/" .. detected_at },
  }
  for key, value in pairs(extra or {}) do
    payload[key] = value
  end
  return {
    queue = "idle-detector.system_idle",
    ts = payload.detected_at,
    payload = payload,
  }
end

local function fresh_idle_event()
  return idle_event({
    detected_at = "2026-06-19T01:00:00Z",
    expires_at = "2026-06-19T01:10:00Z",
  })
end

local function stale_idle_event()
  return idle_event({
    detected_at = "2026-06-19T01:00:00Z",
    expires_at = "2026-06-19T01:20:00Z",
  })
end

local function stale_tick_event(slot)
  local tick_slot = slot or 1782003600000
  return {
    queue = "archaudit.archaudit_tick",
    ts = tick_slot,
    payload = { raiser = "archaudit.audit_poll" },
  }
end

local function mock_env(repo, max_issues)
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = repo or "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$ARCHAUDIT_MAX_ISSUES_PER_IDLE"', { stdout = max_issues or "3", stderr = "", exit_code = 0 })
end

local function observe_facts(opts)
  opts = opts or {}
  local facts = {
    schema_version = opts.schema_version or 1,
    source = opts.source or {
      durable_root = "/tmp/fkst-durable",
      database = "/tmp/fkst-durable/delivery.redb",
      read_semantics = "single read transaction",
      history_semantics = "delivery queue snapshot only",
    },
    limits = opts.limits or { max_deliveries = 500, max_dead_letters = 500 },
    truncated = opts.truncated or { deliveries = false, dead_letters = false },
    queues = opts.queues or {
      { queue = "proposal", depth = 0, pending = 0, in_flight = 0, retrying = 0, oldest_pending_age_ms = nil },
    },
    deliveries = opts.deliveries or json.decode("[]"),
    dead_letters = opts.dead_letters or json.decode("[]"),
  }
  if not opts.omit_generated_at then
    facts.generated_at_ms = opts.generated_at_ms or 1781830860000
  end
  if opts.omit_source then facts.source = nil end
  if opts.omit_limits then facts.limits = nil end
  if opts.omit_truncated then facts.truncated = nil end
  if opts.omit_queues then facts.queues = nil end
  return facts
end

local function mock_observe(snapshot)
  t.mock_observe(snapshot or observe_facts())
end

local function mock_idle_observe()
  mock_observe(observe_facts())
end

local function mock_busy_observe()
  mock_observe(observe_facts({
    queues = {
      { queue = "proposal", depth = 1, pending = 1, in_flight = 0, retrying = 0, oldest_pending_age_ms = 1000 },
    },
  }))
end

local function mock_stale_observe()
  mock_observe(observe_facts({ generated_at_ms = 1781831461000 }))
end

local function mock_idle_observe_at(generated_at_ms)
  mock_observe(observe_facts({ generated_at_ms = generated_at_ms }))
end

local function mock_codex_findings(stdout, exit_code)
  t.mock_command("codex exec", {
    stdout = stdout,
    stderr = exit_code == 0 and "" or "codex timeout",
    exit_code = exit_code or 0,
  })
end

local function finding_json(rule, why)
  return '{"file":"packages/archaudit/core.lua","line":1,"rule":"' .. rule .. '","why":"' .. why .. '","suggested_fix":"Fix ' .. rule .. '."}'
end

local function findings_json(count)
  local rows = {}
  for index = 1, count do
    table.insert(rows, finding_json("Rule" .. tostring(index), "Issue " .. tostring(index) .. "."))
  end
  return "[" .. table.concat(rows, ",") .. "]"
end

local function fake_git(calls)
  return {
    show_file = function(ref, path, timeout)
      table.insert(calls, { ref = ref, path = path, timeout = timeout })
      return { stdout = "line one\n", stderr = "", exit_code = 0 }
    end,
  }
end

local function fake_audit_department(label_stdout, extra_ports)
  local model = github_fake.model()
  local label_calls = {}
  local search_calls = {}
  local git_calls = {}
  local github = github_fake.new(model)
  function github.issue_search(repo, query, fields, timeout)
    table.insert(search_calls, { repo = repo, query = query, fields = fields, timeout = timeout })
    return { stdout = "[]", stderr = "", exit_code = 0 }
  end
  function github.label_list(repo, timeout)
    table.insert(label_calls, { repo = repo, timeout = timeout })
    return { stdout = label_stdout or "[]", stderr = "", exit_code = 0 }
  end
  t.eq(type(audit_main.make_department), "function")
  local ports = { github = github, git = fake_git(git_calls) }
  for key, value in pairs(extra_ports or {}) do
    ports[key] = value
  end
  local dept = audit_main.make_department(ports)
  dept.model = model
  dept.search_calls = search_calls
  dept.git_calls = git_calls
  return dept, model, label_calls
end

local function fake_audit_department_with_search(search_stdout, label_stdout, extra_ports)
  local model = github_fake.model()
  local label_calls = {}
  local search_calls = {}
  local git_calls = {}
  local github = github_fake.new(model)
  function github.issue_search(repo, query, fields, timeout)
    table.insert(search_calls, { repo = repo, query = query, fields = fields, timeout = timeout })
    return { stdout = search_stdout or "[]", stderr = "", exit_code = 0 }
  end
  function github.label_list(repo, timeout)
    table.insert(label_calls, { repo = repo, timeout = timeout })
    return { stdout = label_stdout or "[]", stderr = "", exit_code = 0 }
  end
  local ports = { github = github, git = fake_git(git_calls) }
  for key, value in pairs(extra_ports or {}) do
    ports[key] = value
  end
  local dept = audit_main.make_department(ports)
  dept.search_calls = search_calls
  dept.git_calls = git_calls
  return dept, model, label_calls
end

local function fake_audit_department_with_github(github, extra_ports)
  t.eq(type(audit_main.make_department), "function")
  local ports = { github = github, git = fake_git({}) }
  for key, value in pairs(extra_ports or {}) do
    ports[key] = value
  end
  return audit_main.make_department(ports)
end

local function fake_audit_department_with_observe(observe_facts_fn)
  return fake_audit_department("[]", {
    observe = {
      facts = function()
        return observe_facts_fn()
      end,
    },
  })
end

local function run_fake_at(dept, event, fixed_now_seconds)
  local previous_now = now
  now = function()
    return fixed_now_seconds
  end
  local ok, result = pcall(testing.run_fake, dept, event)
  now = previous_now
  if not ok then
    error(result, 0)
  end
  return result
end

local function run_fake_failure_at(dept, event, fixed_now_seconds)
  local previous_now = now
  now = function()
    return fixed_now_seconds
  end
  local ok, result = pcall(testing.run_fake_expecting_failure, dept, event)
  now = previous_now
  if not ok then
    error(result, 0)
  end
  return result
end

return {
  run_department_opts = run_department_opts,
  idle_event = idle_event,
  fresh_idle_event = fresh_idle_event,
  stale_idle_event = stale_idle_event,
  stale_tick_event = stale_tick_event,
  mock_env = mock_env,
  observe_facts = observe_facts,
  mock_observe = mock_observe,
  mock_idle_observe = mock_idle_observe,
  mock_busy_observe = mock_busy_observe,
  mock_stale_observe = mock_stale_observe,
  mock_idle_observe_at = mock_idle_observe_at,
  mock_codex_findings = mock_codex_findings,
  finding_json = finding_json,
  findings_json = findings_json,
  fake_git = fake_git,
  fake_audit_department = fake_audit_department,
  fake_audit_department_with_search = fake_audit_department_with_search,
  fake_audit_department_with_github = fake_audit_department_with_github,
  fake_audit_department_with_observe = fake_audit_department_with_observe,
  run_fake_at = run_fake_at,
  run_fake_failure_at = run_fake_failure_at,
}
