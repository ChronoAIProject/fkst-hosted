local core = require("core")

local M = {}

M.spec = {
  consumes = { "entity_view_probe" },
  produces = { "entity_view_probe_result" },
}

local function lua_literal(value)
  local kind = type(value)
  if kind == "string" then
    return string.format("%q", value)
  end
  if kind == "number" or kind == "boolean" then
    return tostring(value)
  end
  if kind == "nil" then
    return "nil"
  end
  if kind == "table" then
    local parts = {}
    for key, field in pairs(value) do
      table.insert(parts, "[" .. lua_literal(key) .. "]=" .. lua_literal(field))
    end
    return "{" .. table.concat(parts, ",") .. "}"
  end
  error("unsupported result value type: " .. kind)
end

local function write_file(path, content)
  local dir = tostring(path):match("^(.*)/[^/]+$")
  if dir ~= nil then
    os.execute("mkdir -p " .. string.format("%q", dir))
  end
  local handle = assert(io.open(path, "w"))
  handle:write(content)
  handle:close()
end

function M.run(payload)
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
  return {
    exit_code = result.exit_code,
    stdout = result.stdout,
    stderr = result.stderr,
  }
end

function pipeline(event)
  local payload = event.payload or {}
  if payload.result_path == nil then
    error("entity view probe requires result_path")
  end
  write_file(payload.result_path, "return " .. lua_literal(M.run(payload)) .. "\n")
end

return M
