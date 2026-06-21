local M = {}

local forbidden_gate_globals = {
  "require",
  "debug",
  "_G",
  "load",
  "loadstring",
  "dofile",
  "loadfile",
  "getfenv",
  "setfenv",
  "rawget",
  "rawset",
  "rawequal",
  "setmetatable",
  "getmetatable",
  "os",
  "io",
  "coroutine",
  "package",
}

local safe_string = {
  byte = string.byte,
  char = string.char,
  find = string.find,
  format = string.format,
  gmatch = string.gmatch,
  gsub = string.gsub,
  len = string.len,
  lower = string.lower,
  match = string.match,
  rep = string.rep,
  reverse = string.reverse,
  sub = string.sub,
  upper = string.upper,
}
-- Lua's shared string value metatable may still expose string.dump through
-- ("").dump even though the sandbox string table omits it. With require/load nil,
-- dumped bytecode is inert here; making that unreachable requires a host-owned
-- restricted-load primitive or an isolated Lua state.

local safe_table = {
  concat = table.concat,
  insert = table.insert,
  move = table.move,
  pack = table.pack,
  remove = table.remove,
  sort = table.sort,
  unpack = table.unpack,
}

local gate_cache = {}

local allowed_lineage_fields = {
  proposal_id = true,
  issue_number = true,
  impl_version = true,
  branch = true,
  base_branch = true,
}

local facts_caps = setmetatable({}, { __mode = "k" })
local facts_methods = {}

function facts_methods.reached(self, milestone, opts)
  local caps = facts_caps[self]
  if caps == nil then
    error("std.devloop_gate: invalid facts capability")
  end
  return caps.reached(milestone, opts) == true
end

function facts_methods.lineage_equals(self, field, expected)
  local caps = facts_caps[self]
  if caps == nil then
    error("std.devloop_gate: invalid facts capability")
  end
  return caps.lineage_equals(field, expected) == true
end

local facts_meta = {
  __index = facts_methods,
  __newindex = function()
    error("std.devloop_gate: facts capability is read-only")
  end,
  __metatable = "std.devloop_gate.facts",
}

local function copy_lineage(lineage)
  if lineage == nil then
    return nil
  end
  if type(lineage) ~= "table" or getmetatable(lineage) ~= nil then
    error("std.devloop_gate: lineage must be a plain data table")
  end
  local copied = {}
  for field, required in pairs(lineage) do
    if allowed_lineage_fields[field] ~= true then
      error("std.devloop_gate: unsupported lineage field")
    end
    if required ~= true then
      error("std.devloop_gate: lineage requirements must be positive")
    end
    copied[field] = true
  end
  return copied
end

local function copy_opts(opts)
  if opts == nil then
    return {}
  end
  if type(opts) ~= "table" or getmetatable(opts) ~= nil then
    error("std.devloop_gate: options must be a plain data table")
  end
  local copied = {}
  for key, value in pairs(opts) do
    if key == "domain" or key == "milestone_domain" then
      copied[key] = tostring(value)
    elseif key == "lineage" then
      copied.lineage = copy_lineage(value)
    else
      error("std.devloop_gate: unsupported gate option")
    end
  end
  return copied
end

local function assert_no_smuggled_executable(value, seen)
  local value_type = type(value)
  if value_type == "function" or value_type == "thread" or value_type == "userdata" then
    error("std.devloop_gate: gate spec must be data-only")
  end
  if value_type ~= "table" then
    return
  end
  if getmetatable(value) ~= nil then
    error("std.devloop_gate: gate spec must not carry metatables")
  end
  seen = seen or {}
  if seen[value] then
    return
  end
  seen[value] = true
  for key, nested in pairs(value) do
    assert_no_smuggled_executable(key, seen)
    assert_no_smuggled_executable(nested, seen)
  end
end

local function assert_allowed_keys(value, allowed)
  for key in pairs(value) do
    if allowed[key] ~= true then
      error("std.devloop_gate: gate spec has non-AST fields")
    end
  end
end

local function assert_dense_gate_list(gates)
  if type(gates) ~= "table" or getmetatable(gates) ~= nil then
    error("std.devloop_gate: all gate requires a plain gate list")
  end
  local count = 0
  local max_index = 0
  for key in pairs(gates) do
    if type(key) ~= "number" or key < 1 or key % 1 ~= 0 then
      error("std.devloop_gate: all gate list must use contiguous integer indexes")
    end
    count = count + 1
    if key > max_index then
      max_index = key
    end
  end
  if count == 0 then
    error("std.devloop_gate: all gate list must not be empty")
  end
  if count ~= max_index then
    error("std.devloop_gate: all gate list must be dense")
  end
end

local function copy_dense_gate_list(gates)
  assert_dense_gate_list(gates)
  local copied = {}
  for index = 1, #gates do
    copied[index] = gates[index]
  end
  return copied
end

local function assert_spec_shape(spec, seen)
  if type(spec) ~= "table" or getmetatable(spec) ~= nil then
    error("std.devloop_gate: gate spec must be a plain data table")
  end
  seen = seen or {}
  if seen[spec] then
    return
  end
  seen[spec] = true
  if spec.op == "all" then
    assert_allowed_keys(spec, { op = true, gates = true })
    assert_dense_gate_list(spec.gates)
    for index = 1, #spec.gates do
      assert_spec_shape(spec.gates[index], seen)
    end
    return
  end
  if spec.op == "reached" then
    assert_allowed_keys(spec, { op = true, milestone = true, opts = true })
    if type(spec.milestone) ~= "string" or spec.milestone == "" then
      error("std.devloop_gate: reached gate requires a milestone")
    end
    copy_opts(spec.opts)
    return
  end
  error("std.devloop_gate: unsupported gate operation")
end

local function assert_loaded_gate_spec(spec)
  assert_no_smuggled_executable(spec)
  assert_spec_shape(spec)
  return spec
end

local function gate_module_name(name)
  if type(name) ~= "string" or name:match("^[A-Za-z_][A-Za-z0-9_]*$") == nil then
    error("std.devloop_gate: gate name must be a safe core/gates module segment")
  end
  return "core.gates." .. name
end

local function gate_source_path(name)
  if package == nil or type(package.searchpath) ~= "function" then
    error("std.devloop_gate: package.searchpath is required to load gate definitions")
  end
  local module = gate_module_name(name)
  local path, search_error = package.searchpath(module, package.path)
  if path == nil then
    error("std.devloop_gate: gate definition not found: " .. module .. tostring(search_error or ""))
  end
  return path, module
end

local function sandbox_env()
  return {
    require_reached = M.require_reached,
    all = M.all,
    pairs = pairs,
    ipairs = ipairs,
    type = type,
    tostring = tostring,
    tonumber = tonumber,
    select = select,
    error = error,
    assert = assert,
    string = {
      byte = safe_string.byte,
      char = safe_string.char,
      find = safe_string.find,
      format = safe_string.format,
      gmatch = safe_string.gmatch,
      gsub = safe_string.gsub,
      len = safe_string.len,
      lower = safe_string.lower,
      match = safe_string.match,
      rep = safe_string.rep,
      reverse = safe_string.reverse,
      sub = safe_string.sub,
      upper = safe_string.upper,
    },
    table = {
      concat = safe_table.concat,
      insert = safe_table.insert,
      move = safe_table.move,
      pack = safe_table.pack,
      remove = safe_table.remove,
      sort = safe_table.sort,
      unpack = safe_table.unpack,
    },
  }
end

local function sandboxed_source(source)
  local lines = {
    "local require_reached <const> = require_reached",
    "local all <const> = all",
    "local pairs <const> = pairs",
    "local ipairs <const> = ipairs",
    "local type <const> = type",
    "local tostring <const> = tostring",
    "local tonumber <const> = tonumber",
    "local select <const> = select",
    "local error <const> = error",
    "local assert <const> = assert",
    "local string <const> = string",
    "local table <const> = table",
  }
  for _, name in ipairs(forbidden_gate_globals) do
    lines[#lines + 1] = "local " .. name .. " <const> = nil"
  end
  lines[#lines + 1] = "local _ENV <const> = nil"
  lines[#lines + 1] = "return (function()"
  lines[#lines + 1] = source
  lines[#lines + 1] = "end)()"
  return table.concat(lines, "\n")
end

local function load_gate_source(source, chunk_name)
  if type(load) ~= "function" then
    error("std.devloop_gate: Lua load-with-env is required to load gate definitions")
  end
  local chunk, load_error = load(sandboxed_source(source), chunk_name, "t", sandbox_env())
  if chunk == nil then
    error("std.devloop_gate: gate definition compile failed: " .. tostring(load_error))
  end
  local ok, spec_or_error = pcall(chunk)
  if not ok then
    error("std.devloop_gate: gate definition load failed: " .. tostring(spec_or_error))
  end
  return assert_loaded_gate_spec(spec_or_error)
end

local function reached_opts_for_facts(opts)
  local copied = {}
  if opts.domain ~= nil then
    copied.domain = opts.domain
  end
  if opts.milestone_domain ~= nil then
    copied.milestone_domain = opts.milestone_domain
  end
  return copied
end

local function binding_value(bindings, field)
  if type(bindings) ~= "table" or getmetatable(bindings) ~= nil then
    error("std.devloop_gate: bindings must be a plain data table")
  end
  local value = bindings[field]
  local value_type = type(value)
  if value_type == "nil" then
    return nil
  end
  if value_type == "table" or value_type == "function" or value_type == "thread" or value_type == "userdata" then
    error("std.devloop_gate: binding values must be scalar")
  end
  return value
end

local function lineage_holds(facts, opts, bindings)
  for field, required in pairs(opts.lineage or {}) do
    if required == true then
      local expected = binding_value(bindings, field)
      if expected == nil or not facts:lineage_equals(field, expected) then
        return false
      end
    end
  end
  return true
end

local function eval(spec, facts, bindings)
  if spec.op == "all" then
    for _, child in ipairs(spec.gates or {}) do
      if not eval(child, facts, bindings) then
        return false
      end
    end
    return true
  end
  if spec.op == "reached" then
    local opts = copy_opts(spec.opts)
    if not lineage_holds(facts, opts, bindings) then
      return false
    end
    return facts:reached(spec.milestone, reached_opts_for_facts(opts))
  end
  error("std.devloop_gate: unsupported gate operation")
end

function M.facts(caps)
  if type(caps) ~= "table" or type(caps.reached) ~= "function" or type(caps.lineage_equals) ~= "function" then
    error("std.devloop_gate: facts requires reached and lineage_equals capabilities")
  end
  local object = {}
  facts_caps[object] = {
    reached = caps.reached,
    lineage_equals = caps.lineage_equals,
  }
  return setmetatable(object, facts_meta)
end

function M.require_reached(milestone, opts)
  if type(milestone) ~= "string" or milestone == "" then
    error("std.devloop_gate: milestone is required")
  end
  return {
    op = "reached",
    milestone = milestone,
    opts = copy_opts(opts),
  }
end

function M.all(gates)
  return {
    op = "all",
    gates = copy_dense_gate_list(gates),
  }
end

function M.load_gate(name)
  local path, module = gate_source_path(name)
  local cached = gate_cache[path]
  if cached ~= nil then
    return cached
  end
  if file == nil or type(file.read) ~= "function" then
    error("std.devloop_gate: file.read SDK is required to load gate definitions")
  end
  local source = file.read(path)
  local spec = load_gate_source(source, "@" .. module)
  gate_cache[path] = spec
  return spec
end

if fkst ~= nil and fkst.test ~= nil then
  function M._load_gate_source_for_test(source)
    return load_gate_source(source, "@std.devloop_gate.test")
  end
end

function M.holds(spec, facts, bindings)
  if facts_caps[facts] == nil then
    error("std.devloop_gate: holds requires an opaque facts capability")
  end
  assert_loaded_gate_spec(spec)
  return eval(spec, facts, bindings or {})
end

return M
