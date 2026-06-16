-- std.ports: production port wiring shared by every department on the gh/git
-- Ports & Adapters layer. A department defines make_department(ports) that
-- closes over the injected handles and returns its engine-facing table
-- ({ spec, pipeline }); std.ports.install builds the production handles from the
-- host exec_sync primitive, constructs the department, and exposes
-- make_department so fake-port tests can re-build the same department against
-- std.github_fake / std.git_fake. This removes the per-department
-- production_exec / production_ports copy (DRY: the framework owns the stable
-- common wiring, the script keeps only its business pipeline).
local M = {}

local function production_exec()
  if type(exec_sync) == "function" then
    return exec_sync
  end
  return function()
    error("std.ports: production ports require exec_sync")
  end
end

function M.production_handles()
  local run = production_exec()
  return {
    github = require("std.github").new(run),
    git = require("std.git").new(run),
  }
end

local function validate_department(department)
  if type(department) ~= "table" or type(department.spec) ~= "table" or type(department.pipeline) ~= "function" then
    error("std.ports.install: make_department must return a table with spec and pipeline", 2)
  end
end

function M.install(make_department)
  assert(type(make_department) == "function", "std.ports.install requires a make_department function")
  local department = make_department(M.production_handles())
  validate_department(department)
  department.make_department = make_department
  return department
end

return M
