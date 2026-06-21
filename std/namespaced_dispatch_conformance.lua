local C = {}

local function pattern_escape(value)
  return tostring(value):gsub("([%(%)%.%%%+%-%*%?%[%]%^%$])", "%%%1")
end

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function package_root_from_test(test_module_name)
  local source = package.searchpath(test_module_name, package.path)
  if source == nil then
    error("namespaced-dispatch: cannot resolve test module " .. tostring(test_module_name))
  end
  local relative = tostring(test_module_name):gsub("%.", "/")
  local root = source:match("(.+)/" .. pattern_escape(relative) .. "%.lua$")
  if root == nil then
    error("namespaced-dispatch: cannot infer package root from " .. tostring(source))
  end
  return root
end

local function department_paths(root)
  local result = {}
  local command = "find " .. shell_single_quote(root .. "/departments") .. " -mindepth 2 -maxdepth 2 -name main.lua | sort"
  local find = assert(io.popen(command))
  for path in find:lines() do
    table.insert(result, path:sub(#root + 2))
  end
  if find:close() == false then
    error("namespaced-dispatch: department discovery failed")
  end
  return result
end

local function load_department_spec(root, path)
  local old_pipeline = pipeline
  local module = dofile(root .. "/" .. path)
  pipeline = old_pipeline
  if type(module) ~= "table" or type(module.spec) ~= "table" then
    error("namespaced-dispatch: department spec missing for " .. tostring(path))
  end
  return module.spec
end

local function production_queue_name(package_name, queue)
  local text = tostring(queue)
  if text:find("%.", 1, false) ~= nil then
    return text
  end
  return tostring(package_name) .. "." .. text
end

local fallthrough_needles = {
  "consumed-queue-unrouted",
  "unknown-queue",
  "unknown queue",
  "unsupported-queue",
  "unsupported queue",
  "unsupported event payload",
  "unsupported sync conflict payload",
  "skip-foreign(payload)",
  "skip-foreign(pr)",
  "skip-foreign(proposal_id)",
  "skip-foreign(source_ref)",
}

local function assert_no_fallthrough(package_name, path, queue, err, logs)
  local text = tostring(err or "") .. "\n" .. tostring(logs or "")
  for _, needle in ipairs(fallthrough_needles) do
    if text:find(needle, 1, true) ~= nil then
      error(
        tostring(package_name)
          .. ": production-shaped consumed queue fell through unsupported path for "
          .. tostring(path)
          .. " queue="
          .. tostring(queue)
          .. ": "
          .. text
      )
    end
  end
end

local function run_department_with_logs(t, root, path, event, opts)
  local config = opts or {}
  local run_opts = config.run_opts or opts
  local result = t.run_department(path, event, run_opts)
  t.is_true(type(result) == "table")

  local captured = {}
  local activity = #(result.raises or {})
  local old_log = log
  local originals = {}
  local cleanup = nil
  for _, name in ipairs({
    "raise",
    "exec_sync",
    "exec_argv",
    "spawn_codex",
    "spawn_codex_sync",
    "await_all",
    "with_lock",
    "once",
    "cache_get",
    "cache_set",
  }) do
    originals[name] = _G[name]
    if type(_G[name]) == "function" then
      _G[name] = function(...)
        activity = activity + 1
        return originals[name](...)
      end
    end
  end
  log = {
    info = function(message)
      activity = activity + 1
      table.insert(captured, tostring(message))
    end,
    warn = function(message)
      activity = activity + 1
      table.insert(captured, tostring(message))
    end,
    error = function(message)
      activity = activity + 1
      table.insert(captured, tostring(message))
    end,
  }

  local old_pipeline = pipeline
  local ok, err = pcall(function()
    if type(config.before_replay) == "function" then
      cleanup = config.before_replay(path, event)
    end
    dofile(root .. "/" .. path)
    pipeline(event)
  end)
  pipeline = old_pipeline
  log = old_log
  for name, value in pairs(originals) do
    _G[name] = value
  end
  if type(cleanup) == "function" then
    local cleanup_ok, cleanup_err = pcall(cleanup)
    if not cleanup_ok and ok then
      ok = false
      err = cleanup_err
    end
  end
  return ok, tostring(err or ""), table.concat({
    tostring(result.error or ""),
    tostring(result.stderr or ""),
    table.concat(captured, "\n"),
  }, "\n"), activity
end

function C.assert_all_consumed_queues_route(config)
  local t = assert(config.t, "namespaced-dispatch: missing fkst.test handle")
  local package_name = assert(config.package_name, "namespaced-dispatch: missing package_name")
  local root = config.package_root or package_root_from_test(assert(config.test_module_name, "namespaced-dispatch: missing test_module_name"))
  local payload_for_queue = assert(config.payload_for_queue, "namespaced-dispatch: missing payload_for_queue")
  local opts_for_case = config.opts_for_case

  for _, path in ipairs(department_paths(root)) do
    local spec = load_department_spec(root, path)
    for _, queue in ipairs(spec.consumes or {}) do
      local event = {
        queue = production_queue_name(package_name, queue),
        payload = payload_for_queue(path, queue),
      }
      local opts = nil
      if type(opts_for_case) == "function" then
        opts = opts_for_case(path, queue, event)
      end
      local _ok, err, logs, activity = run_department_with_logs(t, root, path, event, opts)
      assert_no_fallthrough(package_name, path, queue, err, logs)
      if activity == 0 then
        error(
          tostring(package_name)
            .. ": production-shaped consumed queue produced no route activity for "
            .. tostring(path)
            .. " queue="
            .. tostring(queue)
        )
      end
    end
  end
end

return C
