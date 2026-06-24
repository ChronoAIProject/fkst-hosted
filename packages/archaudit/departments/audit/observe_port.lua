local core = require("core")

local M = {}

function M.facts(opts)
  if type(fkst) ~= "table" or type(fkst.observe) ~= "function" then
    error("archaudit: missing-observe: fkst.observe is required")
  end
  local ok, facts = pcall(fkst.observe, opts)
  if not ok then
    local message = tostring(facts)
    if message:find("FKST_DURABLE_ROOT", 1, true) ~= nil then
      error("archaudit: observe-durable-root-unresolved: " .. message)
    end
    if message:find("fkst.observe snapshot", 1, true) ~= nil then
      error("archaudit: observe-malformed: " .. message)
    end
    error("archaudit: observe-unreadable: " .. message)
  end
  return core.validate_observe_facts(facts)
end

return M
