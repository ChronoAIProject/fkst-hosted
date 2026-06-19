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
  test_scan_accepts_production_namespaced_queue = function()
    local ok, err, logs = run_department_with_logs("departments/external_pr_intake/main.lua", {
      queue = "github-external-pr-intake.external_pr_scan",
      payload = {
        schema = "github-external-pr-intake.v1",
      },
    })
    local text = tostring(err or "") .. "\n" .. tostring(logs or "")

    t.eq(ok, false)
    t.is_true(text:find("FKST_GITHUB_REPO is required", 1, true) ~= nil)
    t.is_nil(text:find("unsupported event payload", 1, true))
    t.is_nil(text:find("skip-foreign", 1, true))
  end,

  test_candidate_non_table_payload_fails_closed = function()
    local ok, err, logs = run_department_with_logs("departments/external_pr_intake/main.lua", {
      queue = "external_pr_candidate",
      payload = "foreign-payload",
    })
    local text = tostring(err or "") .. "\n" .. tostring(logs or "")

    t.eq(ok, false)
    t.is_true(text:find("invalid-payload", 1, true) ~= nil)
    t.is_nil(text:find("skip-foreign", 1, true))
  end,
}
