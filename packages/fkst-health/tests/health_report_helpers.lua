-- Fakes for the health_report department: an in-memory filesystem plus injectable
-- probes, so the whole tick can be exercised without touching a real disk, a real
-- engine snapshot, or a real codex.
--
-- The filesystem fake is deliberately faithful about ONE thing: `mv` moves an entry
-- rather than copying it. That is what makes the atomic-publish assertions meaningful
-- -- a test can prove the final name only ever appears through a rename.
local main = require("departments.health_report.main")

local M = {}

M.session = "8f2c1d64-0a1b-4c2d-8e3f-0123456789ab"
M.namespace = "chronoai-fkst"
M.now = 1785650400 -- 2026-08-02T06:00:00Z
M.runtime_root = "/scratch/runtime"
M.health_directory = M.runtime_root .. "/health"

function M.fake_fs()
  local fs = { entries = {}, calls = {} }

  fs.file = {
    write = function(path, text)
      table.insert(fs.calls, { op = "write", path = path })
      fs.entries[path] = text
    end,
    read = function(path)
      if fs.entries[path] == nil then
        error("fake-fs: no such file: " .. tostring(path))
      end
      return fs.entries[path]
    end,
    exists = function(path)
      return fs.entries[path] ~= nil
    end,
    list = function(dir)
      local prefix, out = tostring(dir) .. "/", {}
      for path in pairs(fs.entries) do
        if path:sub(1, #prefix) == prefix then
          table.insert(out, path)
        end
      end
      table.sort(out)
      return out
    end,
  }

  fs.exec_argv = function(request)
    local argv = type(request) == "table" and request.argv or {}
    local command = tostring(argv[1])
    table.insert(fs.calls, { op = command, argv = argv })
    if command == "mkdir" then
      return { stdout = "", stderr = "", exit_code = 0 }
    end
    if command == "mv" then
      local from, to = argv[3], argv[4]
      if fs.entries[from] == nil then
        return { stdout = "", stderr = "missing source", exit_code = 1 }
      end
      fs.entries[to] = fs.entries[from]
      fs.entries[from] = nil
      return { stdout = "", stderr = "", exit_code = 0 }
    end
    if command == "rm" then
      fs.entries[argv[3]] = nil
      return { stdout = "", stderr = "", exit_code = 0 }
    end
    return { stdout = "", stderr = "unsupported argv", exit_code = 1 }
  end

  return fs
end

--- A well-formed, quiet observe snapshot: the schema the department understands, with
--- nothing moving.
function M.observe_facts(overrides)
  local facts = {
    schema_version = 1,
    generated_at_ms = M.now * 1000,
    source = { durable_root = "/durable", database = "deliveries.redb", history_semantics = "retained" },
    limits = { max_deliveries = 100, max_dead_letters = 100 },
    truncated = { deliveries = false, dead_letters = false },
    queues = { { queue = "fkst-health.health_tick", depth = 0, pending = 0, in_flight = 0, retrying = 0 } },
    deliveries = {},
    dead_letters = {},
  }
  for key, value in pairs(overrides or {}) do
    facts[key] = value
  end
  return facts
end

M.default_env = {
  FKST_RUNTIME_ROOT = M.runtime_root,
  FKST_SESSION_ID = M.session,
  FKST_WORK_LABEL_NAMESPACE = M.namespace,
}

--- Build the department with fake ports. `overrides` replaces any port; `overrides.env`
--- replaces the environment table.
function M.department(overrides)
  overrides = overrides or {}
  local fs = overrides.fs or M.fake_fs()
  local cache = overrides.cache or {}
  local env = overrides.env or M.default_env
  local seen_env = {}

  local ports = {
    read_env = function(name)
      table.insert(seen_env, name)
      return env[name]
    end,
    observe = function(...)
      table.insert(fs.calls, { op = "observe", argument_count = select("#", ...) })
      return M.observe_facts()
    end,
    codex_runs = function()
      return { running = {}, recent = {} }
    end,
    commit_count = function()
      return 0
    end,
    run_codex = function(opts)
      table.insert(fs.calls, { op = "codex", opts = opts })
      return { stdout = "The session is quiet.", stderr = "", exit_code = 0 }
    end,
    now = function()
      return M.now
    end,
    cache_get = function(key)
      return cache[key]
    end,
    cache_set = function(key, value)
      cache[key] = value
    end,
    file = fs.file,
    exec_argv = fs.exec_argv,
  }
  for key, value in pairs(overrides) do
    if key ~= "fs" and key ~= "cache" and key ~= "env" then
      ports[key] = value
    end
  end

  local department = main.make_department(ports)
  department.fs = fs
  department.cache = cache
  department.seen_env = seen_env
  return department
end

function M.tick()
  return {
    queue = "fkst-health.health_tick",
    ts = "2026-08-02T06:00:00Z",
    payload = {
      schema = "fkst-health.health-tick.v1",
      slot = "2026-08-02T06:00:00Z",
      source_ref = { kind = "cron", ref = "fkst-health/health_poll/2026-08-02T06:00:00Z" },
    },
  }
end

--- Every published report currently on the fake filesystem, as { path, text } pairs.
function M.reports(department)
  local out = {}
  for path, text in pairs(department.fs.entries) do
    local name = path:match("([^/]+)$") or path
    if name:sub(1, 1) ~= "." and name:sub(-3) == ".md" then
      table.insert(out, { path = path, name = name, text = text })
    end
  end
  table.sort(out, function(left, right)
    return left.path < right.path
  end)
  return out
end

--- The one report a normal tick produces, failing loudly when there is not exactly one.
function M.only_report(department)
  local reports = M.reports(department)
  if #reports ~= 1 then
    error("expected exactly one report, found " .. tostring(#reports))
  end
  return reports[1]
end

function M.front_matter(text)
  local body_at = text:find("\n+++\n", 4, true)
  if body_at == nil then
    error("front matter is not terminated")
  end
  return text:sub(5, body_at)
end

function M.body(text)
  local body_at = text:find("\n+++\n", 4, true)
  if body_at == nil then
    error("front matter is not terminated")
  end
  return text:sub(body_at + 5)
end

return M
