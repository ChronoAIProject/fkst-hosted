-- Which session wrote this comment.
--
-- Every session in a deployment posts as the same GitHub App identity, so a
-- reader cannot tell two concurrent sessions apart from the comment alone.
-- Establishing that a FOREIGN session had written the timeout-redrive comments
-- on an issue (#5750) required correlating GitHub timestamps against kubectl
-- logs from three sandbox pods. That evidence belongs on the issue.
--
-- The env reader is injected rather than required: this module is then pure
-- with respect to the ambient exec, and a test can drive it with a table.
local M = {}

local function trimmed(read_env, name)
  local ok, value = pcall(read_env, name)
  if not ok or value == nil then
    return nil
  end
  local text = tostring(value):gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    return nil
  end
  return text
end

-- Deliberately NOT memoized. A module-local cache would be hidden mutable state
-- in a library that every comment write calls, and it would leak between tests
-- sharing a Lua process. Three env reads per comment is not worth that.
local function identity(read_env)
  return {
    session_id = trimmed(read_env, "FKST_SESSION_ID"),
    namespace = trimmed(read_env, "FKST_WORK_LABEL_NAMESPACE"),
    trigger_issue = trimmed(read_env, "FKST_TRIGGER_ISSUE"),
  }
end

--- The attribution line, or nil when this deployment has no session identity.
--
-- Degrades rather than fails at every step: a comment must never be lost
-- because attribution could not be built. No session id at all means a
-- standalone package deployment, where there is nothing to attribute.
function M.build(read_env, repo)
  if type(read_env) ~= "function" then
    return nil
  end
  local ident = identity(read_env)
  if ident.session_id == nil then
    return nil
  end

  local label = ident.session_id
  if ident.namespace ~= nil then
    label = ident.namespace .. "-" .. ident.session_id
  end

  local target_repo = nil
  if repo ~= nil and tostring(repo) ~= "" then
    target_repo = tostring(repo)
  end
  if ident.trigger_issue ~= nil and target_repo ~= nil then
    return "Written by session: [" .. label .. "](https://github.com/"
      .. target_repo .. "/issues/" .. ident.trigger_issue .. ")"
  end
  -- Known-but-unlinkable is still worth stating; the id alone identifies the pod.
  return "Written by session: " .. label
end

return M
