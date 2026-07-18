-- workflow-writer: the CATALOG seam.
--
-- The injected blueprint provider. It merges the adapter's built-in record(s) with any
-- host files under FKST_WORKFLOW_CATALOG_ROOT and validates BOTH through the one kernel
-- validator (workflow.engine.catalog.validate_records -> blueprint.validate). Duplicate
-- ids disqualify both peers (the kernel's rule), so a host-authored template that
-- collides with the built-in `workflow-authoring-flow` id silently disables both --
-- documented in the README as the id-collision warning.
--
-- The catalog root is also the location where an authored template PR lands its new
-- file, so a subsequent load validates the delivered template identically to any
-- built-in record.
local engine_catalog = require("workflow.engine.catalog")
local records = require("records")

local M = {}

-- Gather the built-in record(s) plus any host catalog files into one array, returning
-- the array and the collection errors so validate_records can attribute file problems.
-- Distinct construction from the security catalog assembler.
local function assemble_catalog(catalog_root)
  local gathered = {}
  local record_list = records.records()
  for index = 1, #record_list do
    gathered[#gathered + 1] = record_list[index]
  end
  local errors = {}
  if type(catalog_root) == "string" and catalog_root ~= "" then
    local collection = engine_catalog.collect_file_records(catalog_root)
    for _, record in ipairs(collection.records or {}) do
      gathered[#gathered + 1] = record
    end
    errors = collection.errors or {}
  end
  return gathered, errors
end

-- deps = { catalog_root }
function M.build(deps)
  local catalog_root = deps and deps.catalog_root

  local provider = {}

  function provider.records()
    return records.records()
  end

  -- Return the set-like table of every id currently present in the validated catalog
  -- (built-ins + host files). The authoring id-collision guard uses this so a drafted
  -- CREATE template can never silently disqualify a template already on disk.
  function provider.catalog_ids()
    local gathered, errors = assemble_catalog(catalog_root)
    local result = engine_catalog.validate_records(gathered, errors)
    local ids = {}
    for id in pairs(result.valid or {}) do
      ids[id] = true
    end
    for _, duplicate in ipairs(result.duplicates or {}) do
      ids[tostring(duplicate.id)] = true
    end
    return ids
  end

  function provider.load_blueprint(_ctx, workflow_id)
    local gathered, errors = assemble_catalog(catalog_root)
    local result = engine_catalog.validate_records(gathered, errors)
    local valid = result.valid[tostring(workflow_id)]
    if valid == nil then
      return nil
    end
    return { blueprint = valid.blueprint }
  end

  return provider
end

return M
