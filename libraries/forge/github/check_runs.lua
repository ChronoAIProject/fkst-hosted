local gitref = require("forge.gitref")

local C = {}

function C.is_open_pr(pr)
  return tostring(pr.state or ""):upper() == "OPEN"
end

function C.check_run_id(run)
  local id = type(run) == "table" and (run.id or run.databaseId or run.database_id) or nil
  local text = tostring(id or "")
  if text ~= "" and text:find("[^0-9]") == nil then
    return text
  end
  return nil
end

function C.check_run_head_sha(run)
  if type(run) ~= "table" then
    return nil
  end
  for _, value in ipairs({
    run.head_sha,
    run.headSha,
    run.headSHA,
  }) do
    if gitref.is_git_sha(value) then
      return tostring(value):lower()
    end
  end
  if type(run.check_suite) == "table" then
    for _, value in ipairs({
      run.check_suite.head_sha,
      run.check_suite.headSha,
    }) do
      if gitref.is_git_sha(value) then
        return tostring(value):lower()
      end
    end
  end
  if type(run.checkSuite) == "table" then
    for _, value in ipairs({
      run.checkSuite.head_sha,
      run.checkSuite.headSha,
    }) do
      if gitref.is_git_sha(value) then
        return tostring(value):lower()
      end
    end
  end
  return nil
end

function C.check_run_name(run)
  if type(run) ~= "table" then
    return ""
  end
  return tostring(run.name or run.context or run.workflowName or run.workflow_name or "")
end

function C.check_run_state(run)
  if type(run) ~= "table" then
    return "", ""
  end
  return tostring(run.state or run.status or ""):upper(), tostring(run.conclusion or ""):upper()
end

local green_required_check_conclusions = {
  SUCCESS = true,
  NEUTRAL = true,
  SKIPPED = true,
}

function C.required_head_check_run_status(runs, head_sha, required_names)
  if type(runs) ~= "table" or not gitref.is_git_sha(head_sha) then
    return "unknown"
  end
  required_names = required_names or {}
  local required = {}
  for _, name in ipairs(required_names) do
    required[tostring(name)] = false
  end
  local expected = tostring(head_sha):lower()
  for _, run in ipairs(runs) do
    local name = C.check_run_name(run)
    if required[name] ~= nil then
      local run_head = C.check_run_head_sha(run)
      if run_head == nil or run_head == expected then
        required[name] = true
        local state, conclusion = C.check_run_state(run)
        if state == "COMPLETED" then
          if not green_required_check_conclusions[conclusion] then
            return "red"
          end
        else
          return "pending"
        end
      end
    end
  end
  for _, name in ipairs(required_names) do
    if required[tostring(name)] ~= true then
      return "unknown"
    end
  end
  return "green"
end

return C
