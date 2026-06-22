local h = require("tests.devloop_helpers")
local t = h.t

local package_root = "packages/github-devloop"

local function read_source(path)
  local handle = assert(io.open(package_root .. "/" .. path, "r"))
  local body = handle:read("*a")
  handle:close()
  return body
end

local function line_count(body)
  local count = 0
  for _ in tostring(body or ""):gmatch("\n") do
    count = count + 1
  end
  return count
end

local function assert_module(path, install_name)
  local body = read_source(path)
  t.is_true(line_count(body) < 700)
  t.is_true(body:find("function M%.install_" .. install_name, 1, false) ~= nil)
  t.is_true(body:find("return M", 1, true) ~= nil)
end

return {
  test_observability_core_is_split_into_department_local_responsibility_modules = function()
    local core_body = read_source("core/observability.lua")
    t.is_true(line_count(core_body) < 250)
    t.is_true(core_body:find('require("departments.observability.census")', 1, true) ~= nil)
    t.is_true(core_body:find('require("departments.observability.common")', 1, true) ~= nil)
    t.is_true(core_body:find('require("departments.observability.dashboard")', 1, true) ~= nil)
    t.is_true(core_body:find('require("departments.observability.reaper")', 1, true) ~= nil)

    assert_module("departments/observability/common.lua", "common")
    assert_module("departments/observability/census.lua", "census")
    assert_module("departments/observability/dashboard.lua", "dashboard")
    assert_module("departments/observability/reaper.lua", "reaper")
  end,
}
