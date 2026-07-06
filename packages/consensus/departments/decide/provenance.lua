local M = {}

function M.verdict_vector(results)
  local vector = {}
  for _, item in ipairs(results or {}) do
    table.insert(vector, {
      angle = tostring(item.angle or "unknown"),
      verdict = item.verdict or "invalid",
    })
  end
  return vector
end

return M
