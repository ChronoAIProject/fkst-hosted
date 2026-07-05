local blueprint = require("core.blueprint")
local fail = require("core.errors").fail

local M = {}

M.MAX_CATALOG_FILES = 128

local function empty_result()
  return {
    valid = {},
    errors = {},
    duplicates = {},
  }
end

local function is_json_path(path)
  return type(path) == "string" and path:sub(-5) == ".json"
end

local function add_error(result, path, why)
  table.insert(result.errors, {
    path = path,
    error = why,
  })
end

local function empty_collection()
  return {
    records = {},
    errors = {},
  }
end

local function duplicate_record(id, records)
  local paths = {}
  for index, record in ipairs(records) do
    paths[index] = record.path
  end
  return {
    id = id,
    paths = paths,
  }
end

local function parse_json_blueprint(source)
  if type(source) ~= "string" then
    return nil, fail("blueprint_json", "not_string", "must be a string")
  end
  local ok, decoded = pcall(json.decode, source)
  if not ok then
    return nil, fail("blueprint_json", "invalid_json", "invalid JSON", {
      error = tostring(decoded),
    })
  end
  return decoded, nil
end

-- Shared JSON→blueprint decode used by BOTH the file source (collect_file_records)
-- and the built-in default source (default_catalog), so host and non-host catalogs
-- are authored in one JSON writing style and flow through one decode + validator.
M.parse_json_blueprint = parse_json_blueprint

function M.collect_file_records(root_dir)
  local collection = empty_collection()
  if type(root_dir) ~= "string" or root_dir == "" then
    add_error(collection, tostring(root_dir), fail("root_dir", "invalid_root_dir", "must be a non-empty string"))
    return collection
  end

  local ok, listed = pcall(file.list, root_dir)
  if not ok then
    add_error(collection, root_dir, fail("root_dir", "file_list_failed", "file.list failed", {
      error = tostring(listed),
    }))
    return collection
  end

  local json_paths = {}
  for _, path in ipairs(listed) do
    if is_json_path(path) then
      table.insert(json_paths, path)
    end
  end
  table.sort(json_paths)
  if #json_paths > M.MAX_CATALOG_FILES then
    add_error(collection, root_dir, fail("catalog", "too_many_files", "catalog exceeds MAX_CATALOG_FILES", {
      max_count = M.MAX_CATALOG_FILES,
      actual_count = #json_paths,
    }))
    return collection
  end

  for _, path in ipairs(json_paths) do
    local read_ok, source = pcall(file.read, path)
    if not read_ok then
      add_error(collection, path, fail("file", "file_read_failed", "file.read failed", {
        error = tostring(source),
      }))
    else
      local decoded, why = parse_json_blueprint(source)
      if decoded == nil then
        add_error(collection, path, why)
      else
        table.insert(collection.records, {
          path = path,
          blueprint = decoded,
        })
      end
    end
  end

  return collection
end

function M.validate_records(records, collection_errors)
  local result = empty_result()
  for _, item in ipairs(collection_errors or {}) do
    table.insert(result.errors, item)
  end

  if type(records) ~= "table" then
    add_error(result, "records", fail("catalog.records", "not_array", "must be an array"))
    return result
  end
  local by_id = {}
  local id_order = {}
  for index, record in ipairs(records) do
    local path = type(record) == "table" and record.path or ("records[" .. tostring(index) .. "]")
    if type(record) ~= "table" then
      add_error(result, path, fail("catalog_record", "not_object", "must be an object"))
    elseif type(record.path) ~= "string" or record.path == "" then
      add_error(result, tostring(path), fail("catalog_record.path", "invalid_path", "must be a non-empty string"))
    else
      local valid, why = blueprint.validate(record.blueprint)
      if not valid then
        add_error(result, record.path, why)
      else
        local parsed = record.blueprint
        if by_id[parsed.id] == nil then
          by_id[parsed.id] = {}
          table.insert(id_order, parsed.id)
        end
        table.insert(by_id[parsed.id], {
          path = path,
          blueprint = parsed,
        })
      end
    end
  end

  for _, id in ipairs(id_order) do
    local records = by_id[id]
    if #records == 1 then
      result.valid[id] = {
        path = records[1].path,
        blueprint = records[1].blueprint,
      }
    else
      local duplicate = duplicate_record(id, records)
      table.insert(result.duplicates, duplicate)
      add_error(result, duplicate.paths[1], fail("catalog." .. id, "duplicate_id", "duplicate workflow id", {
        id = id,
        peers = duplicate.paths,
      }))
    end
  end

  return result
end

function M.load_catalog(root_dir)
  local collection = M.collect_file_records(root_dir)
  return M.validate_records(collection.records, collection.errors)
end

function M.install(target)
  target.catalog = M
end

return M
