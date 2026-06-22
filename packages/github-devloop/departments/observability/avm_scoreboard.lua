local M = {}

function M.install_avm_scoreboard(core)
local task_levels = { "L0", "L1", "L2", "L3", "L4", "unclassified" }
local task_level_set = {
  L0 = true,
  L1 = true,
  L2 = true,
  L3 = true,
  L4 = true,
}

local function number_value(value)
  local parsed = tonumber(value)
  if parsed == nil or parsed < 0 then
    return nil
  end
  return parsed
end

local function int_value(value)
  local parsed = number_value(value)
  if parsed == nil then
    return 0
  end
  return math.floor(parsed)
end

local function optional_int(value)
  local parsed = number_value(value)
  if parsed == nil or parsed ~= math.floor(parsed) then
    return nil
  end
  return parsed
end

local function normalize_task_level(value)
  local text = tostring(value or ""):upper()
  if task_level_set[text] then
    return text
  end
  return "unclassified"
end

local function gate_state(value)
  local text = tostring(value or ""):lower()
  if text == "pass" or text == "passed" or text == "true" or text == "green" or text == "success" then
    return "pass"
  end
  if text == "fail" or text == "failed" or text == "false" or text == "red"
    or text == "failure" or text == "invalid_self_attested" then
    return "fail"
  end
  if text == "pending" or text == "unknown" then
    return "pending"
  end
  return nil
end

local function marker_attr(marker, name)
  return tostring(marker or ""):match(tostring(name) .. '="([^"]*)"')
end

local function comments_from_entity(entity)
  local comments = {}
  local function append_list(values)
    if type(values) ~= "table" then
      return
    end
    for _, comment in ipairs(values) do
      table.insert(comments, comment)
    end
  end

  if type(entity) == "table" then
    append_list(entity.comments)
    if type(entity.parent_issue) == "table" then
      append_list(entity.parent_issue.comments)
    end
    if type(entity.issue) == "table" then
      append_list(entity.issue.comments)
    end
    if type(entity.pr) == "table" then
      append_list(entity.pr.comments)
    end
  end
  return comments
end

local function copy_fact(raw)
  local fact = {}
  if type(raw) ~= "table" then
    return fact
  end
  for key, value in pairs(raw) do
    fact[key] = value
  end
  return fact
end

local function decorate_with_attempt_projection(fact, comments, now_seconds)
  if type(fact) ~= "table" then
    return nil
  end
  if fact.avm_rate_numerator ~= nil and fact.avm_rate_denominator ~= nil then
    return fact
  end
  if fact.repo == nil or fact.issue_number == nil then
    return fact
  end
  local projection = core.autonomy_attempt_projection(comments, fact.repo, fact.issue_number, {
    proposal_id = fact.proposal_id,
    now_seconds = now_seconds,
  })
  if projection.total_attempts > 0 then
    fact.attempt_projection = projection
    fact.attempts = projection.attempts
    fact.attempt_outcomes = projection.outcomes
    fact.avm_rate_numerator = projection.valid_merges
    fact.avm_rate_denominator = projection.total_attempts
  end
  return fact
end

local function fact_from_marker(marker, comment)
  local proposal_id = marker_attr(marker, "proposal")
  local pr_number = marker_attr(marker, "pr")
  local version = marker_attr(marker, "version")
  local head_sha = marker_attr(marker, "head_sha")
  if proposal_id == nil or pr_number == nil or version == nil or head_sha == nil then
    return nil
  end
  return core.autonomy_result_record_from_marker(marker, comment, proposal_id, pr_number, version, head_sha)
end

local function append_comment_facts(facts, comments, now_seconds)
  for _, comment in ipairs(core._trusted_marker_comments(comments)) do
    local body = core._comment_body(comment)
    for marker in body:gmatch("<!%-%- fkst:github%-devloop:autonomy%-result:v1.-%-%->") do
      local fact = fact_from_marker(marker, comment)
      if fact ~= nil then
        table.insert(facts, decorate_with_attempt_projection(fact, comments, now_seconds))
      end
    end
    for marker in body:gmatch("<!%-%- fkst:github%-devloop:merged:v1.-%-%->") do
      if marker:find('autonomy_result="v1"', 1, true) ~= nil then
        local fact = fact_from_marker(marker, comment)
        if fact ~= nil then
          table.insert(facts, decorate_with_attempt_projection(fact, comments, now_seconds))
        end
      end
    end
  end
end

local function append_direct_facts(facts, values)
  if type(values) ~= "table" then
    return
  end
  for _, value in ipairs(values) do
    if type(value) == "table" then
      table.insert(facts, copy_fact(value))
    end
  end
end

local function append_entity_direct_facts(facts, entity)
  if type(entity) ~= "table" then
    return
  end
  if type(entity.autonomy_result) == "table" then
    table.insert(facts, copy_fact(entity.autonomy_result))
  end
  append_direct_facts(facts, entity.avm_facts)
  append_direct_facts(facts, entity.autonomy_facts)
  append_direct_facts(facts, entity.autonomy_results)
end

function core.collect_avm_scoreboard_facts(entities, now_seconds)
  local facts = {}
  for _, entity in ipairs(entities or {}) do
    append_entity_direct_facts(facts, entity)
    append_comment_facts(facts, comments_from_entity(entity), now_seconds)
  end
  return facts
end

local function fact_identity(fact)
  for _, key in ipairs({ "merge_id", "attempt_id", "id" }) do
    local value = fact[key]
    if value ~= nil and tostring(value) ~= "" then
      return key .. ":" .. tostring(value)
    end
  end
  local parts = {}
  for _, key in ipairs({ "proposal_id", "pr_number", "version", "head_sha" }) do
    local value = fact[key]
    if value ~= nil and tostring(value) ~= "" then
      table.insert(parts, tostring(value))
    end
  end
  if #parts >= 2 then
    return "merge:" .. table.concat(parts, "|")
  end
  return nil
end

local function empty_bucket(level)
  return {
    level = level,
    merges = 0,
    avm_numerator = 0,
    avm_denominator = 0,
    cost_total = 0,
    cost_missing = false,
    rounds = {},
    revert_numerator = 0,
    revert_denominator = 0,
    false_consensus_numerator = 0,
    false_consensus_denominator = 0,
  }
end

local function first_int(fact, keys)
  for _, key in ipairs(keys) do
    local parsed = optional_int(fact[key])
    if parsed ~= nil then
      return parsed
    end
  end
  return nil
end

local function first_number(fact, keys)
  for _, key in ipairs(keys) do
    local parsed = number_value(fact[key])
    if parsed ~= nil then
      return parsed
    end
  end
  return nil
end

local function avm_rate_parts(fact)
  local numerator = first_int(fact, { "avm_rate_numerator", "valid_merges" })
  local denominator = first_int(fact, { "avm_rate_denominator", "total_attempts" })
  if numerator ~= nil and denominator ~= nil then
    return numerator, denominator
  end
  if type(fact.attempt_projection) == "table" then
    numerator = first_int(fact.attempt_projection, { "valid_merges", "avm_rate_numerator" })
    denominator = first_int(fact.attempt_projection, { "total_attempts", "avm_rate_denominator" })
    if numerator ~= nil and denominator ~= nil then
      return numerator, denominator
    end
  end
  local valid = tostring(fact.valid_autonomous_merge or ""):lower()
  if valid == "true" or valid == "false" or valid == "pending" or valid == "invalid_self_attested" then
    return valid == "true" and 1 or 0, 1
  end
  return 0, 0
end

local function avm_cost(fact)
  return first_number(fact, { "cost", "total_cost", "cost_units", "codex_calls", "token_cost" })
end

local function nested_gate(fact, names)
  for _, name in ipairs(names) do
    local state = gate_state(fact[name])
    if state ~= nil then
      return state
    end
  end
  if type(fact.gates) == "table" then
    for _, name in ipairs(names) do
      local state = gate_state(fact.gates[name])
      if state ~= nil then
        return state
      end
    end
  end
  return nil
end

local function explicit_false_consensus_parts(fact)
  local numerator = first_int(fact, { "false_consensus_rate_numerator", "false_consensus_numerator" })
  local denominator = first_int(fact, { "false_consensus_rate_denominator", "false_consensus_denominator" })
  if numerator ~= nil and denominator ~= nil then
    return numerator, denominator
  end
  local value = fact.false_consensus
  if value == true then
    return 1, 1
  end
  if value == false then
    return 0, 1
  end
  local text = tostring(value or ""):lower()
  if text == "true" or text == "false" then
    return text == "true" and 1 or 0, 1
  end
  return nil, nil
end

function core.aggregate_avm_scoreboard(facts)
  local buckets = {}
  local seen = {}
  for _, level in ipairs(task_levels) do
    buckets[level] = empty_bucket(level)
  end

  for _, raw in ipairs(facts or {}) do
    if type(raw) == "table" then
      local identity = fact_identity(raw)
      if identity == nil or seen[identity] ~= true then
        if identity ~= nil then
          seen[identity] = true
        end
        local bucket = buckets[normalize_task_level(raw.task_level or raw.task_class or raw.risk_tier)]
        bucket.merges = bucket.merges + 1

        local avm_numerator, avm_denominator = avm_rate_parts(raw)
        bucket.avm_numerator = bucket.avm_numerator + avm_numerator
        bucket.avm_denominator = bucket.avm_denominator + avm_denominator

        local cost = avm_cost(raw)
        if cost == nil then
          bucket.cost_missing = true
        else
          bucket.cost_total = bucket.cost_total + cost
        end

        local rounds = first_int(raw, { "rounds", "median_rounds", "merge_rounds" })
        if rounds ~= nil then
          table.insert(bucket.rounds, rounds)
        end

        local revert_state = nested_gate(raw, { "no_revert_reopen", "gate_no_revert_reopen", "revert", "reopened" })
        if revert_state == "pass" or revert_state == "fail" then
          bucket.revert_denominator = bucket.revert_denominator + 1
          if revert_state == "fail" then
            bucket.revert_numerator = bucket.revert_numerator + 1
          end
        end

        local false_numerator, false_denominator = explicit_false_consensus_parts(raw)
        if false_numerator ~= nil and false_denominator ~= nil then
          bucket.false_consensus_numerator = bucket.false_consensus_numerator + false_numerator
          bucket.false_consensus_denominator = bucket.false_consensus_denominator + false_denominator
        end
      end
    end
  end

  local rows = {}
  for _, level in ipairs(task_levels) do
    table.insert(rows, buckets[level])
  end
  return rows
end

local function format_decimal(value)
  local text = string.format("%.2f", tonumber(value) or 0)
  text = text:gsub("0+$", ""):gsub("%.$", "")
  if text == "" then
    return "0"
  end
  return text
end

local function format_rate(numerator, denominator)
  if tonumber(denominator) == nil or tonumber(denominator) <= 0 then
    return "n/a"
  end
  local pct = (tonumber(numerator) or 0) / tonumber(denominator) * 100
  return tostring(int_value(numerator)) .. "/" .. tostring(int_value(denominator)) .. " (" .. format_decimal(pct) .. "%)"
end

local function format_median(values)
  if type(values) ~= "table" or #values == 0 then
    return "n/a"
  end
  local ordered = {}
  for _, value in ipairs(values) do
    table.insert(ordered, tonumber(value) or 0)
  end
  table.sort(ordered)
  local mid = math.floor(#ordered / 2) + 1
  if #ordered % 2 == 1 then
    return format_decimal(ordered[mid])
  end
  return format_decimal((ordered[mid - 1] + ordered[mid]) / 2)
end

local function format_cost_per_avm(bucket)
  if bucket.merges == 0 then
    return "n/a"
  end
  if bucket.cost_missing then
    return "unknown"
  end
  if bucket.avm_numerator <= 0 then
    return "n/a"
  end
  return format_decimal(bucket.cost_total / bucket.avm_numerator)
end

function core.render_avm_scoreboard_bucket(bucket)
  return "- " .. tostring(bucket.level)
    .. " merges=" .. tostring(bucket.merges)
    .. " AVM-rate=" .. format_rate(bucket.avm_numerator, bucket.avm_denominator)
    .. " cost-per-AVM=" .. format_cost_per_avm(bucket)
    .. " revert-rate=" .. format_rate(bucket.revert_numerator, bucket.revert_denominator)
    .. " median-rounds=" .. format_median(bucket.rounds)
    .. " false-consensus-rate=" .. format_rate(bucket.false_consensus_numerator, bucket.false_consensus_denominator)
end
end

return M
