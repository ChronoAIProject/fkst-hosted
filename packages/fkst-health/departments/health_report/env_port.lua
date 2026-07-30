local env = require("workflow.env")

local M = {}

-- The complete set of environment values this package may read. Every one is
-- non-secret by construction: a session id, a work-label namespace, a scratch path, a
-- work label, and a repository slug.
--
-- FKST_SESSION_CREDS_DIR and FKST_SESSION_DELIVERY_GRANTS are deliberately ABSENT.
-- The evidence this department collects is handed to a codex and rendered into a
-- report that leaves the pod, so the allowlist is the boundary that makes it
-- impossible for a credential to reach either one -- not a convenience.
M.allowed = {
  FKST_SESSION_ID = true,
  FKST_WORK_LABEL_NAMESPACE = true,
  FKST_RUNTIME_ROOT = true,
  FKST_SESSION_WORK_LABEL = true,
  FKST_GITHUB_REPO = true,
}

function M.command_for(name)
  if M.allowed[name] ~= true then
    error("fkst-health: env-name-denied: " .. tostring(name), 0)
  end
  return 'printf %s "$' .. name .. '"'
end

-- propagate_exec_errors stays false on purpose: an unreadable environment value
-- degrades one signal to nil, exactly like a failed probe. Nothing about reading the
-- environment may cost a session its heartbeat.
M.read = env.read_env(M.command_for, { propagate_exec_errors = false })

return M
