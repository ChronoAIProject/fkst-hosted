local core = require("core")

local M = {}

M.spec = {
  consumes = { "test_fresh_issue_view" },
}

function pipeline(event)
  local payload = event.payload or {}
  local result = core.fetch_issue_view_state(payload.repo, payload.number, payload.updated_at, { fresh = true })
  if result.exit_code ~= 0 then
    error("github-devloop: fresh issue view failed: " .. tostring(result.stderr))
  end
end

return M
