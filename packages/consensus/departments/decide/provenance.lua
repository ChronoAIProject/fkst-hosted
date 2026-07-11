local M = {}

function M.verdict_vector(results)
  local vector = {}
  for _, item in ipairs(results or {}) do
    if type(item) == "table" and item.verdict ~= nil then
      table.insert(vector, {
        angle = tostring(item.angle or "unknown"),
        verdict = item.verdict,
      })
    end
  end
  return vector
end

return M
