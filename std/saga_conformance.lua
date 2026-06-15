-- std.saga_conformance: thin test oracle for saga progress and idempotency.
local C = {}

local write_prefixes = {
  "gh issue comment",
  "gh issue edit",
  "gh issue close",
  "gh issue create",
  "gh issue reopen",
  "gh pr merge",
  "gh pr comment",
  "gh pr edit",
  "gh pr create",
  "gh pr close",
  "gh pr ready",
  "gh pr reopen",
  "gh label add",
  "gh label remove",
  "gh label create",
  "gh workflow run",
  "git push",
}

local write_fragments = {
  "gh api --method POST",
  "gh api --method PATCH",
  "gh api --method PUT",
  "gh api --method DELETE",
  "--add-label",
  "--remove-label",
}

local function starts_with(value, prefix)
  return value:sub(1, #prefix) == prefix
end

local function is_graphql_mutation(command)
  return command:find("gh api graphql", 1, true) ~= nil
    and command:find("mutation") ~= nil
end

local function is_git_push(command)
  local first = true
  for token in command:gmatch("%S+") do
    if first then
      if token ~= "git" then
        return false
      end
      first = false
    elseif token == "push" then
      return true
    end
  end
  return false
end

function C.is_write_class(command_string)
  local command = tostring(command_string or "")
  if is_git_push(command) then
    return true
  end
  for _, prefix in ipairs(write_prefixes) do
    if starts_with(command, prefix) then
      return true
    end
  end
  for _, fragment in ipairs(write_fragments) do
    if command:find(fragment, 1, true) ~= nil then
      return true
    end
  end
  if is_graphql_mutation(command) then
    return true
  end
  return false
end

local function command_text(call)
  if type(call) == "table" then
    return tostring(call.rendered or call.cmd or call.command or "") .. "\n" .. tostring(call.stdin or "")
  end
  return call
end

local function count_write_calls(start_index)
  local count = 0
  local calls = fkst.test.command_calls()
  for index = start_index + 1, #calls do
    if C.is_write_class(command_text(calls[index])) then
      count = count + 1
    end
  end
  return count
end

local function count_raises(result)
  if type(result) == "table" and type(result.raises) == "table" then
    return #result.raises
  end
  return 0
end

local function assert_delivery_succeeded(label, ok, result_or_err)
  if not ok then
    error(
      "std.saga_conformance: "
        .. label
        .. " delivery errored; idempotent no-op not proven: "
        .. tostring(result_or_err)
    )
  end
  if type(result_or_err) == "table"
    and result_or_err.exit_code ~= nil
    and tonumber(result_or_err.exit_code) ~= 0 then
    error(
      "std.saga_conformance: "
        .. label
        .. " delivery failed with exit_code="
        .. tostring(result_or_err.exit_code)
        .. "; idempotent no-op not proven"
    )
  end
end

local function validate_case(name, case)
  if type(case) ~= "table" then
    error("std.saga_conformance: " .. name .. " requires case")
  end
end

local function is_non_empty_string(value)
  return type(value) == "string" and value ~= ""
end

local function assert_non_empty_table(name, value)
  if type(value) ~= "table" or #value == 0 then
    error("std.saga_conformance: " .. name .. " requires non-empty table")
  end
end

local function validate_post_condition(step, condition)
  if type(condition) ~= "table" then
    error("std.saga_conformance: step " .. tostring(step.id) .. " has malformed post_condition")
  end
  if not is_non_empty_string(condition.id) then
    error("std.saga_conformance: step " .. tostring(step.id) .. " post_condition requires id")
  end
  if not is_non_empty_string(condition.kind) then
    error("std.saga_conformance: post_condition " .. tostring(condition.id) .. " requires kind")
  end
end

local function validate_step(step)
  if type(step) ~= "table" then
    error("std.saga_conformance: saga step is malformed")
  end
  if not is_non_empty_string(step.id) then
    error("std.saga_conformance: saga step requires id")
  end
  if not is_non_empty_string(step.effect) then
    error("std.saga_conformance: step " .. tostring(step.id) .. " requires effect")
  end
  if not is_non_empty_string(step.request_queue) then
    error("std.saga_conformance: step " .. tostring(step.id) .. " requires request_queue")
  end
  if type(step.post_conditions) ~= "table" or #step.post_conditions == 0 then
    error("std.saga_conformance: step " .. tostring(step.id) .. " requires non-empty post_conditions")
  end
  for _, condition in ipairs(step.post_conditions) do
    validate_post_condition(step, condition)
  end
end

function C.assert_external_effect_saga(saga_def)
  if type(saga_def) ~= "table" then
    error("std.saga_conformance: external-effect saga requires definition")
  end
  if not is_non_empty_string(saga_def.id) then
    error("std.saga_conformance: external-effect saga requires id")
  end
  assert_non_empty_table("external-effect saga steps", saga_def.steps)
  for _, step in ipairs(saga_def.steps) do
    validate_step(step)
  end
end

local function require_fragment(text, fragment, message)
  if tostring(text or ""):find(tostring(fragment or ""), 1, true) == nil then
    error(message)
  end
end

local function assert_graphql_add_blocked_by(condition, evidence)
  local command = tostring(evidence and evidence.command or "")
  require_fragment(
    command,
    "addBlockedBy",
    "std.saga_conformance: post_condition " .. tostring(condition.id) .. " requires addBlockedBy mutation"
  )
  local blocked_field = condition.blocked_field or "issueId"
  local blocking_field = condition.blocking_field or "blockingIssueId"
  require_fragment(
    command,
    blocked_field .. ":",
    "std.saga_conformance: post_condition " .. tostring(condition.id)
      .. " requires GraphQL field " .. tostring(blocked_field)
  )
  require_fragment(
    command,
    blocking_field .. ":",
    "std.saga_conformance: post_condition " .. tostring(condition.id)
      .. " requires GraphQL field " .. tostring(blocking_field)
  )
  for _, forbidden in ipairs(condition.forbidden_fields or {}) do
    if command:find(tostring(forbidden), 1, true) ~= nil then
      error(
        "std.saga_conformance: post_condition " .. tostring(condition.id)
          .. " forbids GraphQL field " .. tostring(forbidden)
      )
    end
  end
end

local function assert_trusted_comment_marker(condition, evidence)
  local body = evidence and evidence.body
  if type(body) ~= "string" or body == "" then
    error(
      "std.saga_conformance: post_condition " .. tostring(condition.id)
        .. " requires marker body evidence"
    )
  end
  if condition.marker ~= nil then
    require_fragment(
      body,
      condition.marker,
      "std.saga_conformance: post_condition " .. tostring(condition.id) .. " requires declared marker"
    )
  end
  for _, fragment in ipairs(condition.required_body_fragments or {}) do
    require_fragment(
      body,
      fragment,
      "std.saga_conformance: post_condition " .. tostring(condition.id)
        .. " requires marker fragment " .. tostring(fragment)
    )
  end
  if evidence.dedup_key ~= nil then
    require_fragment(
      body,
      'dedup="' .. tostring(evidence.dedup_key) .. '"',
      "std.saga_conformance: post_condition " .. tostring(condition.id) .. " requires dedup marker"
    )
  end
  if condition.issue_number_attr ~= nil and evidence.issue_number ~= nil then
    require_fragment(
      body,
      tostring(condition.issue_number_attr) .. '="' .. tostring(evidence.issue_number) .. '"',
      "std.saga_conformance: post_condition " .. tostring(condition.id) .. " requires issue number marker"
    )
  end
end

function C.assert_external_effect_post_condition(condition, evidence)
  if type(condition) ~= "table" then
    error("std.saga_conformance: external-effect post_condition requires definition")
  end
  if condition.kind == "github-add-blocked-by-edge" then
    assert_graphql_add_blocked_by(condition, evidence or {})
    return
  end
  if condition.kind == "trusted-comment-marker" then
    assert_trusted_comment_marker(condition, evidence or {})
    return
  end
  error("std.saga_conformance: unsupported external-effect post_condition kind " .. tostring(condition.kind))
end

function C.assert_progress(_t, case)
  validate_case("assert_progress", case)
  if type(case.first) ~= "function" then
    error("std.saga_conformance: assert_progress requires first")
  end
  local before = #fkst.test.command_calls()
  local result = case.first()
  if count_write_calls(before) + count_raises(result) == 0 then
    error("std.saga_conformance: assert_progress observed no write-class commands")
  end
end

function C.assert_idempotent(_t, case)
  validate_case("assert_idempotent", case)
  if type(case.first) ~= "function" then
    error("std.saga_conformance: assert_idempotent requires first")
  end
  if type(case.second) ~= "function" then
    error("std.saga_conformance: assert_idempotent requires second")
  end
  local before_first = #fkst.test.command_calls()
  local first_result = case.first()
  local first_effects = count_write_calls(before_first) + count_raises(first_result)
  if first_effects == 0 then
    error("std.saga_conformance: assert_idempotent: first delivery made no write-class effect; nothing to prove")
  end
  local before_second = #fkst.test.command_calls()
  local ok, second_result = pcall(case.second)
  assert_delivery_succeeded("second", ok, second_result)
  local second_effects = count_write_calls(before_second) + count_raises(second_result)
  if second_effects ~= 0 then
    error("std.saga_conformance: assert_idempotent observed effects on second delivery")
  end
end

return C
