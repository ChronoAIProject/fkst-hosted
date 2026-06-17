local S = {}
local strings = require("std.strings")
local decimal_checksum = strings.decimal_checksum

function S.install(M)
local default_call_timeout = 10
local default_wall_clock_budget = 90

local function positive_integer(value, fallback, minimum, maximum)
  local n = tonumber(value)
  if n == nil or n ~= math.floor(n) or n < minimum or n > maximum then
    return fallback
  end
  return n
end

function M.sweep_deadline(now_seconds, limits)
  local base = tonumber(now_seconds) or now()
  local budget = positive_integer(limits and limits.wall_clock_budget, default_wall_clock_budget, 1, 3600)
  return base + budget
end

function M.sweep_remaining_seconds(deadline)
  local remaining = math.floor((tonumber(deadline) or 0) - now())
  if remaining < 1 then
    return 0
  end
  return remaining
end

function M.sweep_call_timeout(limits, deadline)
  local configured = positive_integer(limits and limits.call_timeout, default_call_timeout, 1, 300)
  local remaining = M.sweep_remaining_seconds(deadline)
  if remaining == 0 then
    return 0
  end
  if remaining < configured then
    return remaining
  end
  return configured
end

function M.sweep_has_budget(deadline)
  return M.sweep_remaining_seconds(deadline) > 0
end

function M.sweep_deadline_deferred_result(error_class, stderr)
  return {
    deferred = true,
    reason = "deadline",
    error_class = tostring(error_class or "sweep command"),
    stdout = "",
    stderr = tostring(stderr or "sweep deadline exhausted"),
    exit_code = 0,
  }
end

function M.sweep_result_deferred(result)
  return type(result) == "table" and result.deferred == true
end

function M.sweep_exec(cmd_or_opts, limits, deadline, error_class, exec)
  local timeout = M.sweep_call_timeout(limits, deadline)
  if timeout <= 0 then
    return M.sweep_deadline_deferred_result(error_class)
  end
  if type(exec) == "function" then
    local opts
    if type(cmd_or_opts) == "table" then
      opts = {}
      for key, value in pairs(cmd_or_opts) do
        opts[key] = value
      end
      opts.timeout = opts.timeout or timeout
    else
      opts = { cmd = cmd_or_opts, timeout = timeout }
    end
    return exec(opts)
  end
  if type(cmd_or_opts) == "table" and type(cmd_or_opts.run) == "function" then
    return cmd_or_opts.run(cmd_or_opts.timeout or timeout)
  end
  local opts
  if type(cmd_or_opts) == "table" then
    opts = {}
    for key, value in pairs(cmd_or_opts) do
      opts[key] = value
    end
    opts.timeout = opts.timeout or timeout
  else
    opts = { cmd = cmd_or_opts, timeout = timeout }
  end
  return M.gh_exec(opts, nil, exec)
end

function M.sweep_run_cmd(cmd, limits, deadline, error_class, exec)
  local result = M.sweep_exec(cmd, limits, deadline, error_class, exec)
  if M.sweep_result_deferred(result) then
    return result
  end
  if result.exit_code ~= 0 then
    error("github-devloop: " .. tostring(error_class or "sweep command") .. " failed: " .. tostring(result.stderr))
  end
  return result
end

function M.sweep_rotation_seed(event)
  if event and event.ts ~= nil then
    return tostring(event.ts)
  end
  local payload = event and event.payload
  if type(payload) == "table" then
    for _, key in ipairs({ "tick", "generated_at", "ts" }) do
      if payload[key] ~= nil then
        return tostring(payload[key])
      end
    end
  end
  return tostring(math.floor(now() / 60))
end

function M.sweep_rotation_offset(count, seed)
  local n = tonumber(count)
  if n == nil or n <= 0 then
    return 0
  end
  local numeric_seed = tonumber(seed)
  if numeric_seed ~= nil and numeric_seed == math.floor(numeric_seed) then
    return numeric_seed % n
  end
  local hash = decimal_checksum(tostring(seed or ""))
  return tonumber(hash) % n
end

function M.sweep_rotate(items, seed)
  local source = items or {}
  local count = #source
  if count <= 1 then
    local copy = {}
    for _, item in ipairs(source) do
      table.insert(copy, item)
    end
    return copy
  end
  local offset = M.sweep_rotation_offset(count, seed)
  local rotated = {}
  for i = 1, count do
    local index = ((offset + i - 1) % count) + 1
    table.insert(rotated, source[index])
  end
  return rotated
end

function M.sweep_batch(items, seed, cap, default_cap)
  local source = items or {}
  local bounded_cap = positive_integer(cap, default_cap or 25, 1, 1000)
  if #source <= bounded_cap then
    local all_items = {}
    for _, item in ipairs(source) do
      table.insert(all_items, item)
    end
    return all_items, 0
  end
  local rotated = M.sweep_rotate(source, seed)
  local selected = {}
  for i, item in ipairs(rotated) do
    if i > bounded_cap then
      break
    end
    table.insert(selected, item)
  end
  return selected, math.max(0, #source - #selected)
end

function M.sweep_cursor_batch(items, cursor, cap, default_cap)
  local source = items or {}
  local count = #source
  local bounded_cap = positive_integer(cap, default_cap or 25, 1, 1000)
  if count <= bounded_cap then
    local all_items = {}
    for _, item in ipairs(source) do
      table.insert(all_items, item)
    end
    return all_items, 0, 0
  end

  local start = tonumber(cursor) or 0
  if start < 0 or start ~= math.floor(start) then
    start = 0
  end
  start = start % count

  local selected = {}
  for i = 1, bounded_cap do
    local index = ((start + i - 1) % count) + 1
    table.insert(selected, source[index])
  end

  local next_cursor = (start + #selected) % count
  return selected, math.max(0, count - #selected), next_cursor
end

function M.sweep_cursor_advance(cursor, total, processed)
  local count = tonumber(total) or 0
  if count <= 0 or count ~= math.floor(count) then
    return 0
  end
  local start = tonumber(cursor) or 0
  if start < 0 or start ~= math.floor(start) then
    start = 0
  end
  local step = tonumber(processed) or 0
  if step < 0 or step ~= math.floor(step) then
    step = 0
  end
  return (start + step) % count
end

function M.sweep_positive_integer(value, fallback, minimum, maximum)
  return positive_integer(value, fallback, minimum, maximum)
end

end

return S
