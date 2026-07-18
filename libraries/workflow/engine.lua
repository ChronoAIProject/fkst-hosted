-- workflow.engine (facade)
--
-- The general workflow-engine kernel. Re-exports the pure submodules plus the two
-- composition entry points (make_departments, marker.for_namespace) so an adapter needs a
-- single require:
--
--   local engine = require("workflow.engine")
--   local marker = engine.marker.for_namespace("fkst:workflow-security")
--   return engine.make_departments(bindings).materialize_next()
--
-- Submodules are also individually requireable (workflow.engine.frontier, etc.) for
-- callers that want a narrow dependency.
local M = {
  errors = require("workflow.engine.errors"),
  blueprint = require("workflow.engine.blueprint"),
  catalog = require("workflow.engine.catalog"),
  digest = require("workflow.engine.digest"),
  marker = require("workflow.engine.marker"),
  materialization = require("workflow.engine.materialization"),
  frontier = require("workflow.engine.frontier"),
  generator = require("workflow.engine.generator"),
  reconcile = require("workflow.engine.reconcile"),
  departments = require("workflow.engine.departments"),
}

-- Composition entry points.
M.make_departments = M.departments.make_departments
M.for_namespace = M.marker.for_namespace

function M.install(target)
  target.engine = M
  return M
end

return M
