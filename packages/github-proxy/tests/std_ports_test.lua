local ports = require("std.ports")

local function assert_install_rejects(make_department)
  local ok, err = pcall(ports.install, make_department)
  assert(ok == false, "install rejects malformed make_department return")
  assert(tostring(err):find("std.ports.install: make_department must return a table with spec and pipeline", 1, true) ~= nil,
    "install reports the department return-shape contract")
end

return {
  test_install_passes_production_handles_and_exposes_make_department = function()
    local seen
    local make_department = function(p)
      seen = p
      return { spec = { consumes = { "q" } }, pipeline = function() end }
    end

    local dept = ports.install(make_department)

    assert(type(seen) == "table", "make_department receives a ports table")
    assert(type(seen.github) == "table" and type(seen.github.read_issue) == "function", "github handle is the adapter")
    assert(type(seen.git) == "table", "git handle present")
    assert(dept.spec.consumes[1] == "q", "department spec preserved")
    assert(dept.pipeline ~= nil, "department pipeline preserved")
    assert(dept.make_department == make_department, "make_department exposed for fake-port tests")
  end,

  test_production_handles_builds_github_and_git_handles = function()
    local handles = ports.production_handles()
    assert(type(handles.github.read_issue) == "function", "github adapter handle")
    assert(type(handles.git) == "table", "git adapter handle")
  end,

  test_install_rejects_non_function = function()
    assert(not pcall(ports.install, nil), "install requires a make_department function")
  end,

  test_install_rejects_nil_department = function()
    assert_install_rejects(function()
      return nil
    end)
  end,

  test_install_rejects_non_table_department = function()
    assert_install_rejects(function()
      return "department"
    end)
  end,

  test_install_rejects_department_missing_spec = function()
    assert_install_rejects(function()
      return { pipeline = function() end }
    end)
  end,

  test_install_rejects_department_with_non_table_spec = function()
    assert_install_rejects(function()
      return { spec = "spec", pipeline = function() end }
    end)
  end,

  test_install_rejects_department_missing_pipeline = function()
    assert_install_rejects(function()
      return { spec = {} }
    end)
  end,

  test_install_rejects_department_with_non_function_pipeline = function()
    assert_install_rejects(function()
      return { spec = {}, pipeline = "pipeline" }
    end)
  end,
}
