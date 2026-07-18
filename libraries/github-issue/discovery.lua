-- github-issue: the shared platform.discovery + platform.lease seams.
--
-- Everything the workflow-security and workflow-writer discovery modules had in
-- common now lives here: label-scoped scope listing (via github-issue.scopes),
-- the current/terminal/blueprint marker reads (delegated to the injected
-- namespace-bound `marker` table), the frontier ledger, and the lease seam (this
-- family of adapters files fresh artifacts and never mutates a foreign one, so a
-- listed scope — already filtered to the adapter's label — is self-held).
--
-- The ONLY per-package variation is how a `created` materialization fact resolves
-- its durable child result: workflow-security maps it statically to `ready`,
-- workflow-writer resolves it live from the delivered PR's lifecycle. That
-- difference is injected as `deps.resolve_created_fact` and stays in the package.
local label = require("github-issue.label")
local scopes = require("github-issue.scopes")

local M = {}

-- deps = {
--   github, repo, marker, bot_login, label,   -- discovery inputs
--   resolve_created_fact,                      -- fn(fact) -> fact  (per-package differ)
--   log_prefix,                                -- string for log_decision lines
-- }
function M.build(deps)
  label.require(deps and deps.label)
  local marker = deps.marker
  local resolve_created_fact = deps.resolve_created_fact
  local log_prefix = tostring(deps.log_prefix or "github-issue")

  local discovery = {}

  function discovery.list_scopes(_ctx)
    return scopes.list(deps)
  end

  function discovery.origin_of(scope)
    return scope.origin
  end

  function discovery.read_current(scope)
    return { state = scope.state }
  end

  function discovery.latest_terminal(scope, _current, origin)
    return marker.parse_terminal_marker(scope.text, origin)
  end

  function discovery.latest_blueprint(scope, _current, origin)
    return marker.parse_blueprint_marker(scope.text, origin)
  end

  function discovery.materialization_facts(scope, _current, origin)
    local facts = marker.parse_materialization_markers(scope.text, origin)
    if resolve_created_fact then
      for _, fact in ipairs(facts or {}) do
        resolve_created_fact(fact)
      end
    end
    return facts
  end

  function discovery.ledger_for_frontier(_scope, facts)
    return facts
  end

  function discovery.log_decision(scope, origin, from_state, to_state, outcome, reason)
    if type(log) == "table" and type(log.info) == "function" then
      log.info(log_prefix .. " dept=discovery scope=" .. tostring(scope.origin or origin)
        .. " from=" .. tostring(from_state) .. " to=" .. tostring(to_state)
        .. " outcome=" .. tostring(outcome) .. " reason=" .. tostring(reason))
    end
  end

  local lease = {}
  function lease.verify_claim(_scope, _origin)
    return true
  end
  function lease.close_done_origin(_scope, _origin)
    return nil
  end

  return discovery, lease
end

return M
