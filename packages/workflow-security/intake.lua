-- workflow-security: the OWN intake seat (select department handlers).
--
-- This adapter claims work through its OWN label/queue path -- NEVER the dev intake
-- candidate seam (INTAKE_POLICY_SET). It consumes its own security_review_request
-- queue and its own cron tick, discovers open fkst-security-labelled review issues,
-- stamps the security-review blueprint marker on any that lack one, and drives the
-- reconcile tick so the materialize_next department advances the pipeline.
--
-- It carries no engine/branching logic: blueprint selection is the single built-in
-- template, digesting is the kernel digest, marker emission is the kernel marker.
local digest = require("workflow.engine.digest")

local M = {}

local function queue_matches(event, queue)
  local actual = tostring(event and event.queue or "")
  return actual == queue or actual:match("%." .. queue .. "$") ~= nil
end

local function is_open(current)
  return tostring(current and current.state or ""):upper() == "OPEN"
end

local function terminalized(terminal_fact)
  return terminal_fact ~= nil and tostring(terminal_fact.state or "") ~= "blocked"
end

-- deps = { marker, discovery, blueprint, workflow_id, repo,
--          materialization_tick_queue, consumes = { ... } }
function M.build(deps)
  local marker = deps.marker
  local discovery = deps.discovery
  local plan_digest = digest.blueprint_digest(deps.blueprint)

  local function stamp_blueprint(scope, origin)
    local marker_text = marker.build_blueprint_marker(origin, deps.workflow_id, plan_digest)
    if marker_text == nil then
      return
    end
    raise("github-proxy.github_issue_comment_request", {
      schema = "github-proxy.issue-comment.v1",
      repo = scope.repo or deps.repo,
      issue_number = scope.number,
      body = "Security review queued: " .. tostring(deps.workflow_id) .. "\n\n" .. marker_text,
      dedup_key = tostring(origin) .. "/blueprint",
      source_ref = { kind = "workflow-security", ref = tostring(origin) .. "/blueprint" },
    })
  end

  local function drive(scope)
    local origin = discovery.origin_of(scope)
    local current = discovery.read_current(scope)
    if not is_open(current) then
      return
    end
    if terminalized(discovery.latest_terminal(scope, current, origin)) then
      return
    end
    if discovery.latest_blueprint(scope, current, origin) == nil then
      stamp_blueprint(scope, origin)
    end
    raise(deps.materialization_tick_queue, {
      schema = "workflow-security.materialization-tick.v1",
      origin = origin,
      repo = scope.repo or deps.repo,
      issue_number = scope.number,
    })
  end

  local function act(event)
    for _, scope in ipairs(discovery.list_scopes({ event = event }) or {}) do
      drive(scope)
    end
  end

  local function accept(event)
    for _, queue in ipairs(deps.consumes or {}) do
      if queue_matches(event, queue) then
        return true
      end
    end
    return false
  end

  return {
    accept = accept,
    done = function(_event)
      return false
    end,
    act = act,
    name = "security_select",
  }
end

return M
