local t = fkst.test

local function package_root()
  local source = package.searchpath("tests.unsupported_payload_test", package.path)
  return source:match("(.+)/tests/unsupported_payload_test%.lua$")
end

local function run_department_with_logs(path, event)
  local result = t.run_department(path, event)
  t.is_true(type(result) == "table")

  local captured = {}
  local old_log = log

  log = {
    info = function(message)
      table.insert(captured, tostring(message))
    end,
    warn = function(message)
      table.insert(captured, tostring(message))
    end,
    error = function(message)
      table.insert(captured, tostring(message))
    end,
  }

  local old_pipeline = pipeline
  local ok, err = pcall(function()
    dofile(package_root() .. "/" .. path)
    pipeline(event)
  end)
  pipeline = old_pipeline
  log = old_log
  return ok, tostring(err or ""), table.concat({
    tostring(result.error or ""),
    table.concat(captured, "\n"),
  }, "\n")
end

return {
  test_driver_accepts_production_namespaced_queue = function()
    local ok, err, logs = run_department_with_logs("departments/ratchet_migration_driver/main.lua", {
      queue = "github-ratchet-migration-slicer.ratchet_migration_poll",
      payload = {
        schema = "github-ratchet-migration-slicer.ratchet-migration-poll.v1",
        ratchet = "saga-handler",
      },
    })
    local text = tostring(err or "") .. "\n" .. tostring(logs or "")

    t.eq(ok, false)
    t.is_true(text:find("FKST_GITHUB_REPO is required", 1, true) ~= nil)
    t.is_nil(text:find("unsupported event payload", 1, true))
    t.is_nil(text:find("skip-foreign", 1, true))
  end,

  test_driver_skips_non_table_payloads = function()
    for _, payload in ipairs({ false, "foreign-payload", 42 }) do
      local result = t.run_department("departments/ratchet_migration_driver/main.lua", {
        queue = "ratchet_migration_poll",
        payload = payload,
      })

      t.eq(result.exit_code, 0)
      t.eq(#result.raises, 0)
    end
  end,
}
