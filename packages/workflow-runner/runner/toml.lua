-- A deliberately NARROW TOML reader: exactly the shape a workflow definition has.
--
-- The engine gives a package `json.decode` but no TOML decoder, and a workflow
-- definition is `.fkst/workflows/<id>.toml`. Rather than hand-roll a general TOML
-- parser — which would be a large amount of untestable surface for a file with
-- five possible keys — this accepts an ENUMERATED subset and rejects everything
-- else with a line number.
--
-- Accepted, and nothing more:
--
--   # comment                        (whole-line only)
--   [[step]]                         array-of-tables header
--   key = "basic string"             with \\ \" \n \t \r escapes
--   key = """multi-line"""           LITERAL content; a leading newline is trimmed
--   key = 123                        non-negative integer
--   key = true | false
--   key = ["a", "b"]                 array of basic strings, on one line or many
--
-- The multi-line form earns its place: a `task` step's prompt is prose, and
-- forcing it onto one physical line makes definitions unreadable and invites
-- authors to reach for a TOML feature this reader does not have.
--
-- Rejecting the rest is the point. A definition using a TOML feature this does
-- not implement gets a clear "unsupported syntax at line N" instead of being
-- silently misread — and a misread definition runs the wrong commands.

local M = {}

--- Unescape a TOML basic string body.
local function unescape(raw, line_number)
  local out, index, length = {}, 1, #raw
  while index <= length do
    local char = raw:sub(index, index)
    if char == "\\" then
      local escaped = raw:sub(index + 1, index + 1)
      if escaped == "n" then
        out[#out + 1] = "\n"
      elseif escaped == "t" then
        out[#out + 1] = "\t"
      elseif escaped == "r" then
        out[#out + 1] = "\r"
      elseif escaped == '"' or escaped == "\\" then
        out[#out + 1] = escaped
      else
        return nil, ("unsupported escape \\%s at line %d"):format(escaped, line_number)
      end
      index = index + 2
    else
      out[#out + 1] = char
      index = index + 1
    end
  end
  return table.concat(out), nil
end

--- Read one double-quoted basic string starting at `index`.
--- Returns the value, the index just past the closing quote, and an error.
local function read_string(text, index, line_number)
  if text:sub(index, index) ~= '"' then
    return nil, nil, ("expected a quoted string at line %d"):format(line_number)
  end
  local cursor, raw = index + 1, {}
  while cursor <= #text do
    local char = text:sub(cursor, cursor)
    if char == "\\" then
      raw[#raw + 1] = text:sub(cursor, cursor + 1)
      cursor = cursor + 2
    elseif char == '"' then
      local value, err = unescape(table.concat(raw), line_number)
      if err ~= nil then
        return nil, nil, err
      end
      return value, cursor + 1, nil
    else
      raw[#raw + 1] = char
      cursor = cursor + 1
    end
  end
  return nil, nil, ("unterminated string at line %d"):format(line_number)
end

--- Read a single-line array of basic strings.
local function read_array(text, line_number)
  local inner = text:match("^%[(.*)%]%s*$")
  if inner == nil then
    return nil, ("unterminated or multi-line array at line %d"):format(line_number)
  end
  local values, cursor = {}, 1
  inner = inner:gsub("^%s+", "")
  while cursor <= #inner do
    while inner:sub(cursor, cursor):match("%s") do
      cursor = cursor + 1
    end
    if cursor > #inner then
      break
    end
    local value, next_index, err = read_string(inner, cursor, line_number)
    if err ~= nil then
      return nil, err
    end
    values[#values + 1] = value
    cursor = next_index
    while inner:sub(cursor, cursor):match("[%s,]") do
      cursor = cursor + 1
    end
  end
  return values, nil
end

--- Read one scalar or array value.
local function read_value(text, line_number)
  local trimmed = text:gsub("^%s+", ""):gsub("%s+$", "")
  if trimmed:sub(1, 1) == '"' then
    local value, next_index, err = read_string(trimmed, 1, line_number)
    if err ~= nil then
      return nil, err
    end
    if trimmed:sub(next_index):gsub("%s", "") ~= "" then
      return nil, ("trailing content after a string at line %d"):format(line_number)
    end
    return value, nil
  end
  if trimmed:sub(1, 1) == "[" then
    return read_array(trimmed, line_number)
  end
  if trimmed == "true" then
    return true, nil
  end
  if trimmed == "false" then
    return false, nil
  end
  if trimmed:match("^%d+$") then
    return tonumber(trimmed), nil
  end
  return nil, ("unsupported value syntax at line %d"):format(line_number)
end

--- Rewrite every `"""…"""` value as an equivalent single-line basic string.
---
--- Content between the delimiters is taken **literally** — no escape processing,
--- unlike a single-line basic string. That is the simpler rule for the one thing
--- this form exists for, a `task` step's prompt: prose is far likelier to contain
--- a stray backslash than an intended escape, and a bare `"` needs no escaping
--- here because only `"""` terminates.
---
--- TOML trims one immediately-following newline after the opening delimiter, and
--- so does this. Interior characters are re-escaped on the way into the
--- single-line form, so what the reader finally produces is byte-for-byte what
--- the author wrote.
local function fold_multiline(text)
  local out, cursor = {}, 1
  while true do
    local open_start, open_end = text:find('"""', cursor, true)
    if open_start == nil then
      out[#out + 1] = text:sub(cursor)
      return table.concat(out), nil
    end
    local close_start = text:find('"""', open_end + 1, true)
    if close_start == nil then
      local line_number = select(2, text:sub(1, open_start):gsub("\n", "")) + 1
      return nil, ("unterminated multi-line string at line %d"):format(line_number)
    end
    local body = text:sub(open_end + 1, close_start - 1):gsub("^\r?\n", "")
    -- Escape what the single-line reader could not otherwise carry. Backslashes
    -- first: escaping them after the others would double the ones just added.
    body = body
      :gsub("\\", "\\\\")
      :gsub('"', '\\"')
      :gsub("\r", "\\r")
      :gsub("\n", "\\n")
      :gsub("\t", "\\t")
    out[#out + 1] = text:sub(cursor, open_start - 1)
    out[#out + 1] = '"' .. body .. '"'
    cursor = close_start + 3
  end
end

--- Net bracket depth of one line, ignoring brackets inside quoted strings.
---
--- Multi-line strings are already folded away by the time this runs, so the only
--- quoting to skip is a single-line basic string.
local function bracket_delta(line)
  local delta, index, in_string = 0, 1, false
  while index <= #line do
    local char = line:sub(index, index)
    if in_string then
      if char == "\\" then
        index = index + 1
      elseif char == '"' then
        in_string = false
      end
    elseif char == '"' then
      in_string = true
    elseif char == "#" then
      -- A comment cannot open or close an array.
      break
    elseif char == "[" then
      delta = delta + 1
    elseif char == "]" then
      delta = delta - 1
    end
    index = index + 1
  end
  return delta
end

--- Join the continuation lines of a multi-line array onto one physical line.
---
--- A long argv is the normal case for a `run` step, and writing one element per
--- line is how anyone would naturally format it. Folding here keeps the value
--- reader a simple single-line parser instead of teaching it to span lines.
---
--- `[[step]]` headers are left alone: they are matched exactly, and their
--- brackets balance on their own line anyway.
local function fold_arrays(text)
  local out, buffer, depth, line_number, open_line = {}, nil, 0, 0, 0
  for line in (text .. "\n"):gmatch("(.-)\n") do
    line_number = line_number + 1
    local trimmed = line:gsub("^%s+", ""):gsub("%s+$", "")
    if depth > 0 then
      buffer = buffer .. " " .. trimmed
      depth = depth + bracket_delta(trimmed)
      if depth <= 0 then
        out[#out + 1] = buffer
        buffer, depth = nil, 0
      end
    elseif trimmed ~= "[[step]]" and bracket_delta(trimmed) > 0 then
      buffer, depth, open_line = trimmed, bracket_delta(trimmed), line_number
    else
      out[#out + 1] = line
    end
  end
  if buffer ~= nil then
    return nil, ("unterminated array starting at line %d"):format(open_line)
  end
  -- The trailing empty element from the final newline is dropped by the caller's
  -- own line walk, so the join is lossless.
  return table.concat(out, "\n"), nil
end

--- Decode the workflow-definition subset.
---
--- Returns `{ <top-level keys>, step = { {...}, ... } }`.
function M.decode(text)
  if type(text) ~= "string" then
    return nil, "workflow definition is not readable text"
  end
  -- Multi-line basic strings are folded into their single-line equivalent BEFORE
  -- the line walk, so the walk stays a plain line-at-a-time reader. Folding is
  -- lossless: the interior newlines and tabs become their escapes, which the
  -- string reader turns back into the original characters.
  local folded, fold_error = fold_multiline(text)
  if fold_error ~= nil then
    return nil, fold_error
  end
  -- Arrays fold AFTER strings, so a bracket inside a multi-line prompt has
  -- already become an ordinary character in a quoted single-line value and
  -- cannot be mistaken for an array delimiter.
  folded, fold_error = fold_arrays(folded)
  if fold_error ~= nil then
    return nil, fold_error
  end
  local document, current, line_number = { step = {} }, nil, 0
  for line in (folded .. "\n"):gmatch("(.-)\n") do
    line_number = line_number + 1
    local trimmed = line:gsub("^%s+", ""):gsub("%s+$", "")
    if trimmed ~= "" and trimmed:sub(1, 1) ~= "#" then
      if trimmed == "[[step]]" then
        current = {}
        document.step[#document.step + 1] = current
      elseif trimmed:sub(1, 1) == "[" then
        -- Any other table header is a shape this reader does not implement, and
        -- guessing at it would run the wrong steps.
        return nil, ("unsupported table header %q at line %d"):format(trimmed, line_number)
      else
        local key, raw = trimmed:match("^([%w_]+)%s*=%s*(.+)$")
        if key == nil then
          return nil, ("unsupported syntax at line %d"):format(line_number)
        end
        local value, err = read_value(raw, line_number)
        if err ~= nil then
          return nil, err
        end
        local target = current or document
        if target[key] ~= nil then
          return nil, ("duplicate key %q at line %d"):format(key, line_number)
        end
        target[key] = value
      end
    end
  end
  return document, nil
end

return M
