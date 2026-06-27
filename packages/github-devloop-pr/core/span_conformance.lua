local S = {}

local function record(id, message)
  return { id = id, message = message }
end

function S.errors(core)
  local out = {}
  for _, message in ipairs(core.strict_restart_responsibility_contract_errors(core.restart_transition_table())) do
    if tostring(message):find("span_contract", 1, true) ~= nil then
      table.insert(out, record("gspan.span-contract", tostring(message)))
    end
  end
  if type(core.hidden_state_conformance_errors) == "function" then
    for _, message in ipairs(core.hidden_state_conformance_errors()) do
      table.insert(out, record("gspan.hidden-state", tostring(message)))
    end
  end
  return out
end

function S.install(M)
  function M.span_conformance_errors()
    return S.errors(M)
  end
end

return S
