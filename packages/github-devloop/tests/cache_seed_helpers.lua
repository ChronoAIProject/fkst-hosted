local M = {}

M.spec = {
  consumes = { "cache_seed" },
  produces = {},
}

function pipeline(event)
  local payload = event.payload or {}
  cache_set(payload.key, payload.value)
end

return M
