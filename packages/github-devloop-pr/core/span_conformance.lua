local S = {}
local hidden_state_conformance = require("devloop.hidden_state_conformance")
local m_rrc = require("devloop.restart_responsibility_contract")

local function record(id, message)
  return { id = id, message = message }
end

function S.errors(core)
  local out = {}
  for _, message in ipairs(m_rrc.strict_restart_responsibility_contract_errors(core, core.restart_transition_table())) do
    if tostring(message):find("span_contract", 1, true) ~= nil then
      table.insert(out, record("gspan.span-contract", tostring(message)))
    end
  end
  for _, message in ipairs(hidden_state_conformance.hidden_state_conformance_errors(core)) do
    table.insert(out, record("gspan.hidden-state", tostring(message)))
  end
  return out
end

function S.install(M)
  function M.span_conformance_errors()
    return S.errors(M)
  end
end

return S
