local ports = require("std.ports")

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
}
