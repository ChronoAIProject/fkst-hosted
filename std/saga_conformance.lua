-- std.saga_conformance: thin test oracle for saga progress and idempotency.
local C = {}

local write_prefixes = {
  "gh issue comment",
  "gh issue edit",
  "gh issue close",
  "gh issue create",
  "gh pr merge",
  "gh pr comment",
  "gh pr edit",
  "gh pr create",
  "gh pr ready",
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

function C.is_write_class(command_string)
  local command = tostring(command_string or "")
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

local function validate_case(name, case)
  if type(case) ~= "table" then
    error("std.saga_conformance: " .. name .. " requires case")
  end
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
  case.first()
  count_write_calls(before_first)
  local before_second = #fkst.test.command_calls()
  local second_result = case.second()
  local second_effects = count_write_calls(before_second) + count_raises(second_result)
  if second_effects ~= 0 then
    error("std.saga_conformance: assert_idempotent observed effects on second delivery")
  end
end

return C
