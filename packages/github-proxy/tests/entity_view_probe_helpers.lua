local core = require("core")

local M = {}

M.spec = {
  consumes = { "entity_view_probe" },
  produces = { "entity_view_probe_result" },
}

function pipeline(event)
  local payload = event.payload or {}
  local kind = tostring(payload.kind or "issue")
  local result
  if kind == "pr" then
    if payload.named_marker_reader then
      result = core.fetch_marker_pr_view(payload.repo, payload.number, payload.updated_at, {
        consumer = payload.consumer,
      })
    else
      result = core.fetch_pr_view(payload.repo, payload.number, payload.updated_at, {
        consumer = payload.consumer,
        fresh = payload.fresh,
        marker_bearing = payload.marker_bearing,
      })
    end
  else
    if payload.named_marker_reader then
      result = core.fetch_marker_issue_view(payload.repo, payload.number, payload.updated_at, {
        consumer = payload.consumer,
      })
    else
      result = core.fetch_issue_view(payload.repo, payload.number, payload.updated_at, {
        consumer = payload.consumer,
        fresh = payload.fresh,
        marker_bearing = payload.marker_bearing,
      })
    end
  end
  raise("entity_view_probe_result", {
    exit_code = result.exit_code,
    stdout = result.stdout,
    stderr = result.stderr,
  })
end

return M
