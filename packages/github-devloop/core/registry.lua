local S = {}

local function index_name(index_entry)
  if type(index_entry) == "string" then
    return index_entry, nil
  end
  if type(index_entry) ~= "table" then
    return nil, nil
  end
  return index_entry.module, index_entry.key
end

local function assert_sorted_unique(index_module, index)
  if type(index) ~= "table" then
    error("github-devloop: registry index must be a table: " .. tostring(index_module))
  end
  local previous = nil
  local seen = {}
  for position, index_entry in ipairs(index) do
    local name = index_name(index_entry)
    if type(name) ~= "string" or name == "" then
      error("github-devloop: registry index entry must be a non-empty string: " .. tostring(index_module))
    end
    if seen[name] then
      error("github-devloop: duplicate registry index entry " .. name .. " in " .. tostring(index_module))
    end
    if previous ~= nil and previous > name then
      error("github-devloop: registry index is not sorted: " .. tostring(index_module))
    end
    seen[name] = true
    previous = name
    if index[position + 1] == nil then
      break
    end
  end
end

local function base_module(index_module)
  return tostring(index_module):gsub("%.index$", "")
end

local function entry_name(entry, module_name, key_field)
  if type(entry) ~= "table" then
    error("github-devloop: registry entry must return a table: " .. tostring(module_name))
  end
  local name = entry[key_field]
  if type(name) ~= "string" or name == "" then
    error("github-devloop: registry entry missing " .. tostring(key_field) .. ": " .. tostring(module_name))
  end
  return name
end

local function require_entry(module_name, M, helpers)
  local loaded = require(module_name)
  if type(loaded) == "function" then
    return loaded(M, helpers or {})
  end
  return loaded
end

function S.load_indexed_array(index_module, key_field, M, helpers)
  local index = require(index_module)
  assert_sorted_unique(index_module, index)
  local rows = {}
  local seen = {}
  local base = base_module(index_module)
  for _, index_entry in ipairs(index) do
    local name, expected_key = index_name(index_entry)
    if type(name) ~= "string" or name == "" then
      error("github-devloop: registry index entry must declare a module: " .. tostring(index_module))
    end
    local module_name = base .. "." .. name
    local entry = require_entry(module_name, M, helpers)
    local key = entry_name(entry, module_name, key_field)
    if expected_key == nil then
      expected_key = name
    end
    if key ~= expected_key then
      error("github-devloop: registry entry key " .. key .. " does not match index entry " .. expected_key)
    end
    if seen[key] then
      error("github-devloop: duplicate registry entry key " .. key)
    end
    seen[key] = true
    table.insert(rows, entry)
  end
  return rows
end

function S.load_indexed_map(index_module, key_field, M, helpers)
  local rows = S.load_indexed_array(index_module, key_field, M, helpers)
  local map = {}
  for _, row in ipairs(rows) do
    local key = row[key_field]
    row[key_field] = nil
    map[key] = row
  end
  return map
end

function S.load_indexed_installers(index_module, M)
  local index = require(index_module)
  assert_sorted_unique(index_module, index)
  local base = base_module(index_module)
  for _, index_entry in ipairs(index) do
    local name = index_name(index_entry)
    if type(name) ~= "string" or name == "" then
      error("github-devloop: registry index entry must declare a module: " .. tostring(index_module))
    end
    local module_name = base .. "." .. name
    local installer = require(module_name)
    if type(installer) ~= "function" then
      error("github-devloop: registry installer must return a function: " .. module_name)
    end
    installer(M)
  end
end

return S
