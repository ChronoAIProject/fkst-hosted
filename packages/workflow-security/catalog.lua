-- workflow-security: the CATALOG seam.
--
-- The injected blueprint provider. It merges the adapter's built-in record(s) with
-- any host files under FKST_WORKFLOW_CATALOG_ROOT and validates BOTH through the one
-- kernel validator (workflow.engine.catalog.validate_records -> blueprint.validate).
-- Duplicate ids disqualify both peers (the kernel's rule), so a host-authored
-- template that collides with the built-in `security-review` id silently disables
-- both — documented in the README as the id-collision warning.
local engine_catalog = require("workflow.engine.catalog")
local records = require("records")

local M = {}

local function merged_records(catalog_root)
  local all = {}
  for _, record in ipairs(records.records()) do
    table.insert(all, record)
  end
  local collection_errors = {}
  if type(catalog_root) == "string" and catalog_root ~= "" then
    local collection = engine_catalog.collect_file_records(catalog_root)
    for _, record in ipairs(collection.records or {}) do
      table.insert(all, record)
    end
    collection_errors = collection.errors or {}
  end
  return all, collection_errors
end

-- deps = { catalog_root }
function M.build(deps)
  local catalog_root = deps and deps.catalog_root

  local provider = {}

  function provider.records()
    return records.records()
  end

  function provider.load_blueprint(_ctx, workflow_id)
    local all, collection_errors = merged_records(catalog_root)
    local result = engine_catalog.validate_records(all, collection_errors)
    local valid = result.valid[tostring(workflow_id)]
    if valid == nil then
      return nil
    end
    return { blueprint = valid.blueprint }
  end

  return provider
end

return M
