-- github-issue: the shippable conformance rule.
--
-- A github-issue-driven package MUST declare, in its host-readable
-- `[github].work_labels` manifest mirror, the same label its Lua discovery
-- actually searches. This function is forwarded from each package's
-- `[conformance].function` (concatenated with the package's other obligations),
-- so `fkst-framework conformance` — hence CI — FAILs when:
--   * no work label is declared/used (empty or non-string), or
--   * the declared `[github].work_labels` array omits the used label (drift).
--
-- Records match the engine's `{ id, message }` HostCheck shape (see
-- devloop/saga_conformance.lua). `errors` returns `{}` when the package is clean.
local label = require("github-issue.label")

local M = {}

local function record(id, message)
  return { id = id, message = tostring(message) }
end

-- args = { used = <the Lua label constant the discovery searches>,
--          declared = <the [github].work_labels array from the fkst.toml> }
function M.errors(args)
  local used = args and args.used
  local declared = args and args.declared
  local out = {}

  if type(used) ~= "string" or used == "" then
    out[#out + 1] = record(
      "github.work-label.missing",
      "a github-issue package must use a non-empty work label (github-issue.label.require)"
    )
    return out
  end

  if not label.declaration_ok(declared, used) then
    out[#out + 1] = record(
      "github.work-label.undeclared",
      "the used work label " .. string.format("%q", used)
        .. " is not present in [github].work_labels; declare it so the host can auto-discover it"
    )
  end

  return out
end

return M
