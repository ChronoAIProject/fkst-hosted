-- workflow-security: the single binding table.
--
-- This composes the four kernel seams (executor / completion / catalog / platform)
-- and the adapter's OWN intake, then hands them to engine.make_departments. It is
-- the only place that touches the GitHub port + ambient runners; the department
-- main.lua files are ~3-line wrappers that each invoke exactly ONE of the returned
-- lazy closures (so saga.department's _G.pipeline mutation never clobbers a sibling).
--
-- It requires the shared kernel by a LOCAL name (engine) and the adapter modules by
-- their bare package-root names -- never require("core"), never _G reads for policy.
local engine = require("workflow.engine")
local codex = require("workflow.codex")
local env = require("workflow.env")
local strings = require("contract.strings")
local github_factory = require("devloop.github_factory")

local security_logic = require("security_logic")
local records = require("records")
local discovery_mod = require("github-issue.discovery")
local executor_mod = require("executor")
local completion_mod = require("completion")
local catalog_mod = require("catalog")
local intake_mod = require("intake")

local function env_command(name)
  return 'printf %s "$' .. name .. '"'
end

local function read_env(name)
  local ok, value = pcall(env.read_env, name, exec_sync, env_command)
  if not ok or type(value) ~= "string" then
    return nil
  end
  return strings.trim(value)
end

local function github_handle()
  local ok, handle = pcall(github_factory.production_handle)
  if not ok then
    return nil
  end
  return handle
end

local function spawn_analysis(prompt)
  return spawn_codex_sync(codex.judgment_codex_opts(prompt, "."))
end

local marker = engine.marker.for_namespace(security_logic.NAMESPACE)
local repo = read_env("FKST_GITHUB_REPO") or ""
local bot_login = read_env("FKST_GITHUB_BOT_LOGIN") or ""

-- The security-specific resolver for a `created` materialization fact: a created
-- marker is only ever written by the executor AFTER a codex step's output
-- validated, so "created" == "ready". (The generic listing/marker/lease seams now
-- live in github-issue.discovery; only this per-package differ stays here.)
local function decorate_created(fact)
  if type(fact) == "table" and fact.state == "created" then
    fact.child_ref = {
      kind = "analysis",
      slot = fact.slot,
      child_issue = fact.child_issue,
      result = { state = "ready" },
    }
  end
  return fact
end

local discovery, lease = discovery_mod.build({
  github = github_handle(),
  repo = repo,
  marker = marker,
  bot_login = bot_login,
  label = security_logic.LABEL,
  resolve_created_fact = decorate_created,
  log_prefix = "workflow-security",
})

local executor = executor_mod.build({
  marker = marker,
  repo = repo,
  spawn_analysis = spawn_analysis,
  final_step_id = records.FINAL_STEP_ID,
  label_available = true,
  max_requests = 20,
})

local catalog = catalog_mod.build({
  catalog_root = read_env("FKST_WORKFLOW_CATALOG_ROOT"),
})

local platform = {
  with_lock = function(_key, fn)
    return fn()
  end,
  lock_key = function(scope)
    return security_logic.NAMESPACE .. "/" .. tostring(scope.origin)
  end,
  exec = exec_sync,
  discovery = discovery,
  lease = lease,
}

local intake_handlers = intake_mod.build({
  marker = marker,
  discovery = discovery,
  blueprint = records.BLUEPRINT,
  workflow_id = security_logic.WORKFLOW_ID,
  repo = repo,
  materialization_tick_queue = security_logic.MATERIALIZATION_TICK_QUEUE,
  consumes = { security_logic.TICK_QUEUE },
})

-- The kernel config, in the exact shape engine.make_departments consumes. The
-- department wrappers build saga.department directly from the kernel handlers this
-- config wraps (reconcile.handlers / the intake handler set) so each main.lua is
-- saga-shaped for the G10 saga-handler ratchet while copying zero engine logic.
local config = {
  namespace = security_logic.NAMESPACE,
  executor = executor,
  completion = { reader = completion_mod.reader },
  catalog = catalog,
  platform = platform,
  package = "workflow-security",
  tick_queue = security_logic.MATERIALIZATION_TICK_QUEUE,
  materialize_next = {
    consumes = { security_logic.MATERIALIZATION_TICK_QUEUE },
    produces = {
      "github-issue-effect.github_issue_create_request",
      "github-comment-effect.github_issue_comment_request",
    },
  },
  intake = {
    consumes = { security_logic.TICK_QUEUE },
    produces = {
      security_logic.MATERIALIZATION_TICK_QUEUE,
      "github-comment-effect.github_issue_comment_request",
    },
    handlers = function()
      return intake_handlers
    end,
  },
  dead_letter = { package = "workflow-security" },
}

local M = {}

-- The seams table reconcile.handlers consumes (namespace + the four seams + tick).
function M.seams()
  return {
    namespace = config.namespace,
    executor = config.executor,
    completion = config.completion,
    catalog = config.catalog,
    platform = config.platform,
    tick_queue = config.tick_queue,
  }
end

-- The OWN-intake handler set (security_select department).
function M.intake_handlers()
  return intake_handlers
end

-- The lazy per-department closures, for callers that prefer the facade entry point.
function M.make_departments()
  return engine.make_departments(config)
end

return M
