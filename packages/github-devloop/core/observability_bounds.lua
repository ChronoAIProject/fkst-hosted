local S = {}

function S.install(M)
local default_observability_list_page_cap = 2
local default_observability_entity_cap = 25
local default_observability_call_timeout = 10
local default_observability_wall_clock_budget = 90

local function positive_integer(value, fallback, minimum, maximum)
  local n = tonumber(value)
  if n == nil or n ~= math.floor(n) or n < minimum or n > maximum then
    return fallback
  end
  return n
end

function M.observability_limits()
  return {
    list_page_cap = default_observability_list_page_cap,
    entity_cap = default_observability_entity_cap,
    call_timeout = default_observability_call_timeout,
    wall_clock_budget = default_observability_wall_clock_budget,
  }
end

function M.observability_deadline(now_seconds, limits)
  local base = tonumber(now_seconds) or now()
  local budget = positive_integer(limits and limits.wall_clock_budget, default_observability_wall_clock_budget, 1, 3600)
  return base + budget
end

function M.observability_remaining_seconds(deadline)
  local remaining = math.floor((tonumber(deadline) or 0) - now())
  if remaining < 1 then
    return 0
  end
  return remaining
end

function M.observability_call_timeout(limits, deadline)
  local configured = positive_integer(limits and limits.call_timeout, default_observability_call_timeout, 1, 300)
  local remaining = M.observability_remaining_seconds(deadline)
  if remaining == 0 then
    return 0
  end
  if remaining < configured then
    return remaining
  end
  return configured
end

function M.observability_has_budget(deadline)
  return M.observability_remaining_seconds(deadline) > 0
end

function M.observability_exec(cmd, limits, deadline, error_class, exec)
  local timeout = M.observability_call_timeout(limits, deadline)
  if timeout <= 0 then
    error("github-devloop: " .. tostring(error_class or "gh observability command") .. " failed: observability deadline exhausted")
  end
  return M.gh_exec({ cmd = cmd, timeout = timeout }, nil, exec)
end

function M.observability_run_cmd(cmd, limits, deadline, error_class, exec)
  local result = M.observability_exec(cmd, limits, deadline, error_class, exec)
  if result.exit_code ~= 0 then
    error("github-devloop: " .. tostring(error_class or "gh observability command") .. " failed: " .. tostring(result.stderr))
  end
  return result
end

function M.observability_rotation_seed(event)
  local payload = event and event.payload
  if type(payload) == "table" then
    for _, key in ipairs({ "tick", "cursor", "round", "generated_at", "ts" }) do
      if payload[key] ~= nil then
        return tostring(payload[key])
      end
    end
  end
  if event and event.ts ~= nil then
    return tostring(event.ts)
  end
  return tostring(now())
end

function M.observability_rotation_offset(count, seed)
  local n = tonumber(count)
  if n == nil or n <= 0 then
    return 0
  end
  local hash = M._decimal_checksum(tostring(seed or ""))
  return tonumber(hash) % n
end

function M.observability_rotate(items, seed)
  local source = items or {}
  local count = #source
  if count <= 1 then
    local copy = {}
    for _, item in ipairs(source) do
      table.insert(copy, item)
    end
    return copy
  end
  local offset = M.observability_rotation_offset(count, seed)
  local rotated = {}
  for i = 1, count do
    local index = ((offset + i - 1) % count) + 1
    table.insert(rotated, source[index])
  end
  return rotated
end

function M.observability_batch(items, seed, cap)
  local source = items or {}
  local bounded_cap = positive_integer(cap, default_observability_entity_cap, 1, 1000)
  local rotated = M.observability_rotate(source, seed)
  local selected = {}
  for i, item in ipairs(rotated) do
    if i > bounded_cap then
      break
    end
    table.insert(selected, item)
  end
  return selected, math.max(0, #source - #selected)
end

function M.observability_entity_candidates(issue_numbers, pr_numbers, seed, cap)
  local candidates = {}
  for _, number in ipairs(issue_numbers or {}) do
    table.insert(candidates, {
      kind = "issue",
      number = number,
      key = string.format("issue/%012d", tonumber(number) or 0),
    })
  end
  for _, number in ipairs(pr_numbers or {}) do
    table.insert(candidates, {
      kind = "pr",
      number = number,
      key = string.format("pr/%012d", tonumber(number) or 0),
    })
  end
  table.sort(candidates, function(a, b)
    return tostring(a.key or "") < tostring(b.key or "")
  end)
  local selected, deferred = M.observability_batch(candidates, seed, cap)
  return selected, deferred
end

function M.observability_sorted_numbers(items)
  local numbers = {}
  local seen = {}
  for _, item in ipairs(items or {}) do
    local number = tonumber(item and item.number)
    local state = tostring(item and item.state or ""):lower()
    if number ~= nil and number >= 1 and number % 1 == 0 and state == "open" and not seen[number] then
      seen[number] = true
      table.insert(numbers, number)
    end
  end
  table.sort(numbers)
  return numbers
end

function M.observability_list_issue_candidates(repo, labels, limits, deadline)
  local items = {}
  for _, label in ipairs(labels or {}) do
    for page = 1, limits.list_page_cap do
      local issue_list = M.observability_run_cmd(
        M.gh_issue_list_observe_cmd(repo, label, page),
        limits,
        deadline,
        "gh observability issue list"
      )
      local parsed = M.parse_issue_list_observe(issue_list.stdout)
      for _, issue in ipairs(parsed) do
        table.insert(items, issue)
      end
      if #parsed < 100 then
        break
      end
    end
  end
  return items
end

function M.observability_list_pr_candidates(repo, limits, deadline)
  local items = {}
  for page = 1, limits.list_page_cap do
    local pr_list = M.observability_run_cmd(
      M.gh_pr_list_observe_cmd(repo, page),
      limits,
      deadline,
      "gh observability PR list"
    )
    local parsed = M.parse_pr_list_observe(pr_list.stdout)
    for _, pr in ipairs(parsed) do
      table.insert(items, pr)
    end
    if #parsed < 100 then
      break
    end
  end
  return items
end

function M.observability_deferred_log_line(fields)
  return table.concat({
    "github-devloop",
    "dept=observability",
    "tag=OBSERVE_DEFERRED",
    "reason=" .. tostring(fields and fields.reason or "batch-cap"),
    "listed_issues=" .. tostring(fields and fields.listed_issues or 0),
    "listed_prs=" .. tostring(fields and fields.listed_prs or 0),
    "processed_issues=" .. tostring(fields and fields.processed_issues or 0),
    "processed_prs=" .. tostring(fields and fields.processed_prs or 0),
    "deferred_issues=" .. tostring(fields and fields.deferred_issues or 0),
    "deferred_prs=" .. tostring(fields and fields.deferred_prs or 0),
    "entity_cap=" .. tostring(fields and fields.entity_cap or 0),
  }, " ")
end

end

return S
