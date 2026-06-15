local M = {}

local function capture_raises(fn)
  local old_raise = raise
  local raised = {}
  raise = function(queue, payload)
    table.insert(raised, {
      queue = queue,
      payload = payload,
    })
  end
  local ok, result = pcall(fn)
  raise = old_raise
  if ok then
    return result, raised, nil
  end
  return nil, raised, { error = result }
end

local function writes_from_department(dept)
  local ports = type(dept) == "table" and dept.ports or nil
  local model = type(dept) == "table" and dept.model or nil
  if type(model) == "table" and type(model.writes) == "table" then
    return model.writes
  end
  if type(ports) == "table" then
    for _, port in pairs(ports) do
      if type(port) == "table" and type(port._model) == "table" and type(port._model.writes) == "table" then
        return port._model.writes
      end
    end
  end
  return {}
end

function M.run_fake(dept, event)
  assert(type(dept) == "table", "dept must be a table")
  assert(type(dept.pipeline) == "function", "dept must expose .pipeline")
  local result, raises, failure = capture_raises(function()
    return dept.pipeline(event)
  end)
  return {
    result = result,
    raises = raises,
    writes = writes_from_department(dept),
    failure = failure,
  }
end

return M
