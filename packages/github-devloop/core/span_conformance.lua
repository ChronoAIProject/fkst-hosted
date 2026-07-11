local S = {}
local hidden_state_conformance = require("devloop.hidden_state_conformance")
local issue_observation_conformance = require("devloop.restart.issue_observation_conformance")
local m_rrc = require("devloop.restart_responsibility_contract")

local START_WORDS = {
  start = true,
  starts = true,
  started = true,
  begin = true,
  begins = true,
  began = true,
  beginning = true,
}

local function sorted_keys(map)
  local keys = {}
  for key, _ in pairs(map or {}) do
    table.insert(keys, key)
  end
  table.sort(keys)
  return keys
end

local function line_number(source, index)
  local line = 1
  for pos = 1, math.max(1, (index or 1) - 1) do
    if source:sub(pos, pos) == "\n" then
      line = line + 1
    end
  end
  return line
end

local function unescape_lua_string(value)
  return tostring(value or ""):gsub('\\"', '"'):gsub("\\'", "'"):gsub("\\n", "\n")
end

local function is_word_char(ch)
  return ch ~= nil and ch:match("[%w_]") ~= nil
end

local function has_start_word(text)
  local lower = tostring(text or ""):lower()
  local pos = 1
  while pos <= #lower do
    local first, last, word = lower:find("([%w_]+)", pos)
    if first == nil then
      return false
    end
    if START_WORDS[word] then
      local before = first > 1 and lower:sub(first - 1, first - 1) or nil
      local after = last < #lower and lower:sub(last + 1, last + 1) or nil
      if not is_word_char(before) and not is_word_char(after) then
        return true
      end
    end
    pos = last + 1
  end
  return false
end

local function parse_quoted_at(source, pos)
  local quote = source:sub(pos, pos)
  if quote ~= '"' and quote ~= "'" then
    return nil
  end
  local out = {}
  local index = pos + 1
  while index <= #source do
    local ch = source:sub(index, index)
    if ch == "\\" and index < #source then
      table.insert(out, source:sub(index, index + 1))
      index = index + 2
    elseif ch == quote then
      return table.concat(out), index + 1
    else
      table.insert(out, ch)
      index = index + 1
    end
  end
  return nil
end

local function quoted_strings(source)
  local strings = {}
  local pos = 1
  while pos <= #source do
    local next_quote = source:find("[\"']", pos)
    if next_quote == nil then
      break
    end
    local value, next_pos = parse_quoted_at(source, next_quote)
    if value ~= nil then
      table.insert(strings, { value = value, start = next_quote })
      pos = next_pos
    else
      pos = next_quote + 1
    end
  end
  return strings
end

local function key_value_strings(source)
  local values = {}
  local pos = 1
  while pos <= #source do
    local start_pos, end_pos, key = source:find("([A-Za-z0-9_]+)%s*=%s*", pos)
    if start_pos == nil then
      break
    end
    local value, next_pos = parse_quoted_at(source, end_pos + 1)
    if value ~= nil then
      values[key] = unescape_lua_string(value)
      pos = next_pos
    else
      pos = end_pos + 1
    end
  end
  return values
end

local function comment_strings(sources)
  local values = {}
  for _, path in ipairs(sorted_keys(sources)) do
    if path == "libraries/devloop/strings.lua" or path:sub(-#"core/strings.lua") == "core/strings.lua" then
      for key, value in pairs(key_value_strings(sources[path])) do
        values[key] = value
      end
    end
  end
  return values
end

local function function_blocks(source)
  local declarations = {}
  local pos = 1
  local line_no = 1
  while pos <= #source + 1 do
    local line_end = source:find("\n", pos, true) or (#source + 1)
    local line = source:sub(pos, line_end - 1)
    local local_name, local_params, local_end =
      line:match("^%s*local%s+function%s+([A-Za-z_][A-Za-z0-9_%.:]*)%s*%(([^)]*)%)()")
    local name = local_name
    local params = local_params
    local header_end = local_end
    if name == nil then
      local direct_name, direct_params, direct_end =
        line:match("^%s*function%s+([A-Za-z_][A-Za-z0-9_%.:]*)%s*%(([^)]*)%)()")
      name = direct_name
      params = direct_params
      header_end = direct_end
    end
    if name ~= nil then
      table.insert(declarations, {
        name = name,
        params = params or "",
        start = pos,
        body_start = pos + header_end - 1,
        start_line = line_no,
      })
    end
    pos = line_end + 1
    line_no = line_no + 1
  end

  local blocks = {}
  for index, declaration in ipairs(declarations) do
    local next_start = declarations[index + 1] and declarations[index + 1].start or (#source + 1)
    table.insert(blocks, {
      name = declaration.name,
      params = declaration.params,
      body = source:sub(declaration.body_start, next_start - 1),
      start_line = declaration.start_line,
    })
  end
  return blocks
end

local function has_head_sha_dependency(block)
  for param in tostring(block.params or ""):gmatch("[^,]+") do
    if param:match("^%s*(.-)%s*$") == "head_sha" then
      return true
    end
  end
  return tostring(block.body or ""):find("head_sha", 1, true) ~= nil
end

local function comment_string_calls(body)
  local calls = {}
  local pos = 1
  while pos <= #body do
    local start_pos, end_pos = body:find("%f[%w_]comment_string%s*%(", pos)
    if start_pos == nil then
      break
    end
    local arg_start = end_pos + 1
    while body:sub(arg_start, arg_start):match("%s") do
      arg_start = arg_start + 1
    end
    local _, first_end = body:find("[A-Za-z_][A-Za-z0-9_%.:]*%s*,%s*", arg_start)
    if first_end ~= nil then
      arg_start = first_end + 1
      while body:sub(arg_start, arg_start):match("%s") do
        arg_start = arg_start + 1
      end
    end
    local value, next_pos = parse_quoted_at(body, arg_start)
    if value ~= nil and value:match("^[A-Za-z0-9_]+$") then
      local close_pos = next_pos
      while body:sub(close_pos, close_pos):match("%s") do
        close_pos = close_pos + 1
      end
      if body:sub(close_pos, close_pos) == ")" then
        table.insert(calls, value)
      end
      pos = next_pos
    else
      pos = end_pos + 1
    end
  end
  return calls
end

local function completion_fact_name_messages(sources)
  local strings = comment_strings(sources)
  local messages = {}
  for _, path in ipairs(sorted_keys(sources)) do
    if path:sub(-4) == ".lua" then
      for _, block in ipairs(function_blocks(sources[path])) do
        if block.name:find("comment_request", 1, true) ~= nil and has_head_sha_dependency(block) then
          for _, key in ipairs(comment_string_calls(block.body)) do
            local text = strings[key] or key
            if has_start_word(key) or has_start_word(text) then
              table.insert(messages, string.format(
                "%s:%d %s completion/output comment uses start wording key %q while requiring post-work field head_sha",
                path,
                block.start_line,
                block.name,
                key
              ))
            end
          end
          for _, literal in ipairs(quoted_strings(block.body)) do
            if has_start_word(unescape_lua_string(literal.value)) then
              table.insert(messages, string.format(
                "%s:%d %s completion/output comment uses start wording literal while requiring post-work field head_sha",
                path,
                block.start_line,
                block.name
              ))
              break
            end
          end
        end
      end
    end
  end
  return messages
end

local function short_function_name(name)
  local value = tostring(name or "")
  return value:match("([^%.:]+)$") or value
end

local function function_index(sources)
  local index = {}
  for _, path in ipairs(sorted_keys(sources)) do
    for _, block in ipairs(function_blocks(sources[path])) do
      local short = short_function_name(block.name)
      index[short] = index[short] or {}
      table.insert(index[short], block)
    end
  end
  return index
end

local function call_names(body)
  local calls = {}
  for start_pos, name in tostring(body or ""):gmatch("()([A-Za-z_][A-Za-z0-9_%.:]*)%s*%(") do
    if tostring(body):sub(math.max(1, start_pos - 9), start_pos - 1) ~= "function " then
      table.insert(calls, short_function_name(name))
    end
  end
  return calls
end

local function marker_helper_name(durable_start_marker)
  local family = tostring(durable_start_marker or ""):match("^([^%s:]+)")
  if family == nil or family == "state" then
    return nil
  end
  return family:gsub("-", "_") .. "_marker"
end

local function state_marker_value(durable_start_marker)
  local prefix = "state:v1 "
  local marker = tostring(durable_start_marker or "")
  if marker:sub(1, #prefix) ~= prefix then
    return nil
  end
  local value = marker:sub(#prefix + 1):match("^%s*(.-)%s*$")
  return value ~= "" and value or nil
end

local function body_mentions_marker(body, durable_start_marker)
  body = tostring(body or "")
  if body:find(tostring(durable_start_marker or ""), 1, true) ~= nil then
    return true
  end
  local helper = marker_helper_name(durable_start_marker)
  if helper ~= nil and body:find("%f[%w_]" .. helper .. "%s*%(") ~= nil then
    return true
  end
  local state = state_marker_value(durable_start_marker)
  if state == nil then
    return false
  end
  if body:find("state_marker%s*%([^%)]*[\"']" .. state .. "[\"']") ~= nil then
    return true
  end
  if body:find("has_state_marker%s*%([^%)]*[\"']" .. state .. "[\"']") ~= nil then
    return true
  end
  if body:find("current_entity_state", 1, true) == nil then
    return false
  end
  return body:find("%f[%w_][A-Za-z_][A-Za-z0-9_]*%.state%s*[=~]=%s*[\"']" .. state .. "[\"']") ~= nil
end

local function function_binds_marker(functions, function_name, durable_start_marker)
  local pending = { function_name }
  local seen = {}
  while #pending > 0 do
    local current = table.remove(pending)
    if not seen[current] then
      seen[current] = true
      for _, block in ipairs(functions[current] or {}) do
        if body_mentions_marker(block.body, durable_start_marker) then
          return true
        end
        for _, callee in ipairs(call_names(block.body)) do
          if not seen[callee] and functions[callee] ~= nil then
            table.insert(pending, callee)
          end
        end
      end
    end
  end
  return false
end

local function worker_rows(transition_sources)
  local rows = {}
  for _, path in ipairs(sorted_keys(transition_sources)) do
    local source = transition_sources[path]
    for start_pos, _quote, state in source:gmatch("()from_state%s*=%s*([\"'])(.-)%2.-state_kind%s*=%s*([\"'])worker%4") do
      table.insert(rows, { state = state, path = path, line = line_number(source, start_pos), start = start_pos })
    end
  end
  return rows
end

local function restart_rows(transition_sources)
  local rows = {}
  for _, path in ipairs(sorted_keys(transition_sources)) do
    local source = transition_sources[path]
    for start_pos, _quote, state in source:gmatch("()from_state%s*=%s*([\"'])(.-)%2") do
      table.insert(rows, { state = state, path = path, line = line_number(source, start_pos), start = start_pos })
    end
  end
  return rows
end

local function span_contracts(transition_sources)
  local contracts = {}
  for _, row in ipairs(restart_rows(transition_sources)) do
    local source = transition_sources[row.path]
    local contract_start, body_start = source:find("span_contract%s*=%s*span_contract%s*%(%s*%{", row.start)
    if contract_start ~= nil then
      local body_end = source:find("%}%s*%)", body_start + 1)
      local body = body_end ~= nil and source:sub(body_start + 1, body_end - 1) or source:sub(body_start + 1)
      local fields = key_value_strings(body)
      if fields.department and fields.durable_start_marker and fields.spawn_predecessor then
        contracts[row.state] = {
          state = row.state,
          department = fields.department,
          durable_start_marker = fields.durable_start_marker,
          spawn_predecessor = fields.spawn_predecessor,
          spawn_function = fields.spawn_function,
          path = row.path,
          line = line_number(source, contract_start),
        }
      end
    end
  end
  return contracts
end

local function department_spawn_sources(department_sources, department)
  local needle = "/departments/" .. tostring(department or "") .. "/"
  local selected = {}
  for path, source in pairs(department_sources or {}) do
    if path:find(needle, 1, true) ~= nil then
      selected[path] = source
    end
  end
  return selected
end

local function predecessor_call_before(source, function_name, index)
  local found = -1
  local pos = 1
  while pos < index do
    local start_pos, end_pos = source:find(function_name, pos, true)
    if start_pos == nil or start_pos >= index then
      break
    end
    local before = start_pos > 1 and source:sub(start_pos - 1, start_pos - 1) or ""
    local after_pos = end_pos + 1
    while source:sub(after_pos, after_pos):match("%s") do
      after_pos = after_pos + 1
    end
    if not is_word_char(before) and source:sub(after_pos, after_pos) == "("
      and source:sub(math.max(1, start_pos - 9), start_pos - 1) ~= "function " then
      found = start_pos
    end
    pos = end_pos + 1
  end
  return found
end

local function contains_codex_dispatch(source)
  return source:find("%f[%w_]spawn_codex_sync%s*%(") ~= nil
    or source:find("%f[%w_]dispatch%s*%(") ~= nil
end

local function function_contains_spawn(source, function_name)
  for _, block in ipairs(function_blocks(source)) do
    if short_function_name(block.name) == function_name and contains_codex_dispatch(block.body) then
      return true
    end
  end
  return false
end

local function function_call_positions(source, function_name)
  local positions = {}
  local pos = 1
  while pos <= #source do
    local start_pos, end_pos = source:find(function_name, pos, true)
    if start_pos == nil then
      break
    end
    local before = start_pos > 1 and source:sub(start_pos - 1, start_pos - 1) or ""
    local after_pos = end_pos + 1
    while source:sub(after_pos, after_pos):match("%s") do
      after_pos = after_pos + 1
    end
    if not is_word_char(before) and source:sub(after_pos, after_pos) == "("
      and source:sub(math.max(1, start_pos - 9), start_pos - 1) ~= "function " then
      table.insert(positions, start_pos)
    end
    pos = end_pos + 1
  end
  return positions
end

local function spawn_positions(source)
  local positions = {}
  for start_pos in source:gmatch("()%f[%w_]spawn_codex_sync%s*%(") do
    table.insert(positions, start_pos)
  end
  for start_pos in source:gmatch("()%f[%w_]dispatch%s*%(") do
    table.insert(positions, start_pos)
  end
  table.sort(positions)
  return positions
end

local function spawn_start_messages(transition_sources, department_sources, support_sources)
  local contracts = span_contracts(transition_sources)
  local functions = function_index(support_sources or department_sources)
  local messages = {}
  for _, row in ipairs(restart_rows(transition_sources)) do
    local contract = contracts[row.state]
    if contract ~= nil and contract.department:sub(1, #"external:") ~= "external:" then
      if not function_binds_marker(functions, contract.spawn_predecessor, contract.durable_start_marker) then
        table.insert(messages, string.format(
          "%s:%d span start predecessor %q does not bind durable start marker %q",
          contract.path,
          contract.line,
          contract.spawn_predecessor,
          contract.durable_start_marker
        ))
      end
      local sources = department_spawn_sources(department_sources, contract.department)
      if next(sources) == nil then
        table.insert(messages, string.format(
          "%s:%d span_contract department %q has no scanned department source",
          contract.path,
          contract.line,
          contract.department
        ))
      else
        local saw_spawn = false
        for _, source_path in ipairs(sorted_keys(sources)) do
          local source = sources[source_path]
          if contract.spawn_function ~= nil then
            if function_contains_spawn(source, contract.spawn_function) then
              saw_spawn = true
              for _, call_pos in ipairs(function_call_positions(source, contract.spawn_function)) do
                if predecessor_call_before(source, contract.spawn_predecessor, call_pos) < 0 then
                  table.insert(messages, string.format(
                    "%s:%d %s call must be preceded by span start predecessor %q for durable start marker %q",
                    source_path,
                    line_number(source, call_pos),
                    contract.spawn_function,
                    contract.spawn_predecessor,
                    contract.durable_start_marker
                  ))
                end
              end
            end
          else
            for _, spawn_pos in ipairs(spawn_positions(source)) do
              saw_spawn = true
              if predecessor_call_before(source, contract.spawn_predecessor, spawn_pos) < 0 then
                table.insert(messages, string.format(
                  "%s:%d codex dispatch must be preceded by span start predecessor %q for durable start marker %q",
                  source_path,
                  line_number(source, spawn_pos),
                  contract.spawn_predecessor,
                  contract.durable_start_marker
                ))
              end
            end
          end
        end
        if not saw_spawn then
          table.insert(messages, string.format(
            "%s:%d span_contract department %q has no codex dispatch call",
            contract.path,
            contract.line,
            contract.department
          ))
        end
      end
    end
  end
  return messages
end

local function normalize_path(path)
  return tostring(path or ""):gsub("\\", "/"):gsub("/+$", "")
end

local function strip_suffix(path, suffix)
  local value = normalize_path(path)
  if value:sub(-#suffix) == suffix then
    return value:sub(1, #value - #suffix)
  end
  return nil
end

local function debug_source_path(fn)
  if type(debug) ~= "table" or type(debug.getinfo) ~= "function" then
    return nil
  end
  local info = debug.getinfo(fn or 1, "S")
  local source = info and info.source or ""
  if source:sub(1, 1) == "@" then
    return normalize_path(source:sub(2))
  end
  return nil
end

local function current_package_root()
  return strip_suffix(debug_source_path(1), "/core/span_conformance.lua")
end

local function parent_dir(path)
  return normalize_path(path):match("^(.*)/[^/]+$")
end

local function devloop_library_root()
  local ok, base = pcall(require, "devloop.base")
  if not ok or type(base) ~= "table" then
    return nil
  end
  return strip_suffix(debug_source_path(base.install), "/base.lua")
end

local function repo_root_from_package_root(owner_root)
  local package_parent = parent_dir(owner_root)
  if package_parent == nil then
    return nil
  end
  if package_parent:sub(-#"/.fkst/local-packages") == "/.fkst/local-packages" then
    return parent_dir(parent_dir(package_parent))
  end
  if package_parent:match("/packages$") then
    return parent_dir(package_parent)
  end
  return nil
end

local function add_source_root(out, seen, actual, logical)
  if actual == nil or logical == nil then
    return
  end
  if logical ~= "packages" and logical ~= "libraries" then
    return
  end
  local normalized = normalize_path(actual)
  if normalized:find("/.fkst/", 1, true) ~= nil then
    return
  end
  if seen[normalized] then
    return
  end
  seen[normalized] = true
  table.insert(out, { actual = normalized, logical = logical })
end

local function source_roots()
  local out = {}
  local seen = {}
  local owner_root = current_package_root()
  local repo_root = repo_root_from_package_root(owner_root)
  if repo_root == nil and file.exists("packages") and file.exists("libraries") then
    repo_root = "."
  end
  if repo_root ~= nil then
    add_source_root(out, seen, repo_root .. "/packages", "packages")
    add_source_root(out, seen, repo_root .. "/libraries", "libraries")
  else
    add_source_root(out, seen, parent_dir(owner_root), "packages")
  end

  local devloop_root = devloop_library_root()
  add_source_root(out, seen, parent_dir(devloop_root), "libraries")
  return out
end

local function path_suffix(path, prefix)
  path = normalize_path(path)
  prefix = normalize_path(prefix)
  if path == prefix then
    return ""
  end
  local rooted_prefix = prefix .. "/"
  if path:sub(1, #rooted_prefix) == rooted_prefix then
    return path:sub(#rooted_prefix + 1)
  end
  return nil
end

local function actual_source_path(roots, source_path)
  for _, root in ipairs(roots or {}) do
    local suffix = path_suffix(source_path, root.logical)
    if suffix ~= nil then
      return suffix == "" and root.actual or (root.actual .. "/" .. suffix)
    end
  end
  return source_path
end

local function should_scan_source(suffix)
  local value = normalize_path(suffix)
  return value:sub(-4) == ".lua"
    and value:find("/tests/", 1, true) == nil
    and value:sub(1, #"tests/") ~= "tests/"
end

local function collect_source_paths()
  local paths = {}
  local seen = {}
  for _, root in ipairs(source_roots()) do
    if file.exists(root.actual) then
      for _, actual in ipairs(file.list(root.actual)) do
        local suffix = path_suffix(actual, root.actual)
        if suffix ~= nil and should_scan_source(suffix) then
          local logical = root.logical .. "/" .. suffix
          if not seen[logical] then
            seen[logical] = true
            table.insert(paths, logical)
          end
        end
      end
    end
  end
  table.sort(paths)
  return paths
end

local function read_sources(paths)
  local sources = {}
  local roots = source_roots()
  for _, path in ipairs(paths or {}) do
    local actual_path = actual_source_path(roots, path)
    if file.exists(actual_path) then
      sources[path] = file.read(actual_path)
    end
  end
  return sources
end

local function partition_sources(sources)
  local transition_sources = {}
  local department_sources = {}
  for path, source in pairs(sources or {}) do
    if path:find("/core/restart/transitions/", 1, true) ~= nil
      or path:find("/restart/issue/transitions/", 1, true) ~= nil then
      transition_sources[path] = source
    end
    if path:find("/departments/", 1, true) ~= nil then
      department_sources[path] = source
    end
  end
  return transition_sources, department_sources
end

local function record(id, message)
  return { id = id, message = message }
end

local function span_declaration_errors(core)
  local out = {}
  for _, message in ipairs(issue_observation_conformance.errors(core.restart_transition_table())) do
    table.insert(out, record("gspan.issue-observation-facts", tostring(message)))
  end
  for _, message in ipairs(m_rrc.strict_restart_responsibility_contract_errors(core, core.restart_transition_table())) do
    if tostring(message):find("span_contract", 1, true) ~= nil then
      table.insert(out, record("gspan.span-contract", tostring(message)))
    end
  end
  for _, message in ipairs(hidden_state_conformance.hidden_state_conformance_errors(core)) do
    table.insert(out, record("gspan.hidden-state", tostring(message)))
  end
  return out
end

function S.errors_from_sources(sources)
  local out = {}
  local transition_sources, department_sources = partition_sources(sources)
  for _, message in ipairs(completion_fact_name_messages(sources)) do
    table.insert(out, record("gspan.wording", message))
  end
  for _, message in ipairs(spawn_start_messages(transition_sources, department_sources, sources)) do
    table.insert(out, record("gspan.spawn-order", message))
  end
  return out
end

function S.source_paths()
  return collect_source_paths()
end

function S.errors(core, paths)
  local out = span_declaration_errors(core)
  for _, error_record in ipairs(S.errors_from_sources(read_sources(paths or S.source_paths()))) do
    table.insert(out, error_record)
  end
  return out
end

function S.install(M)
  function M.span_conformance_errors()
    return S.errors(M)
  end
end

S._completion_fact_name_messages = completion_fact_name_messages
S._spawn_start_messages = spawn_start_messages
S._span_declaration_errors = span_declaration_errors
S._source_roots = source_roots

return S
