local M = {}
local issue = require("std.github.issue")

local function copy(value)
  if type(value) ~= "table" then
    return value
  end
  local result = {}
  for key, field in pairs(value) do
    result[copy(key)] = copy(field)
  end
  return result
end

function M.model(seed)
  return {
    issues = seed and seed.issues or {},
    writes = seed and seed.writes or {},
  }
end

function M.new(model)
  assert(type(model) == "table", "std.github_fake.new requires a model")
  local handle = { _model = model }
  function handle._exec(argv, timeout, context)
    table.insert(model.writes, {
      kind = "exec",
      argv = copy(argv),
      timeout = timeout,
      context = context,
    })
    return { stdout = "", stderr = "", exit_code = 0 }
  end
  function handle.read_issue(source_ref)
    local fixture = model.issues[source_ref.ref]
    if fixture == nil then
      error("fake: unknown issue " .. tostring(source_ref.ref))
    end
    return copy(issue.normalize_issue(fixture, source_ref))
  end
  return handle
end

return M
