-- github-issue: the work-label invariant.
--
-- A GitHub-issue-driven adapter claims its work through a single declared label
-- (e.g. "fkst-security"). This module is the one place that invariant is
-- enforced: `require` asserts a package handed a non-empty label into the shared
-- discovery, so a package that composes github-issue with no label fails to boot
-- rather than silently searching a hard-coded default. `declaration_ok` is the
-- helper the shippable conformance rule uses to pin the runtime label constant to
-- the host-readable `[github].work_labels` manifest mirror.
local M = {}

-- The runtime assert. `value` is `deps.label` from the package's discovery build.
-- Returns the validated label so callers can `local l = label.require(deps)`.
function M.require(value)
  assert(
    type(value) == "string" and value ~= "",
    "github-issue: a work label is required (deps.label must be a non-empty string)"
  )
  return value
end

-- `declared` is the `[github].work_labels` array from the package fkst.toml;
-- `used` is the Lua label constant the discovery actually searches. True when the
-- host-visible declaration contains the label the adapter really polls.
function M.declaration_ok(declared, used)
  if type(declared) ~= "table" or type(used) ~= "string" or used == "" then
    return false
  end
  for _, l in ipairs(declared) do
    if l == used then
      return true
    end
  end
  return false
end

return M
