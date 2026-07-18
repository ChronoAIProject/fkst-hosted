-- workflow.engine.reconcile
--
-- The generalized reconcile control-flow, extracted from the reference package's
-- materialize_reconcile.process_origin loop with EVERY dev/platform coupling severed.
-- It reads NOTHING ambient: no require("devloop.*"), no _G, no ambient exec_sync/with_lock.
-- All boundary I/O arrives through four injected seams + a namespace token.
--
-- ===========================================================================
-- SEAM CONTRACT (adapters MUST implement exactly these shapes)
-- ---------------------------------------------------------------------------
-- reconcile.handlers(seams) -> { accept, done, act, wrap, name }   (a saga handler set)
--
-- seams = {
--   namespace  = "<token>",       -- marker namespace, e.g. "fkst:workflow-security"
--   tick_queue = "<queue>",       -- the tick queue this department consumes
--   dept       = "<name>",        -- department name (optional; default below)
--   wrap       = function(name, raw) ... end,   -- optional pipeline-failure wrapper
--   executor   = <EXECUTOR>,
--   completion = <COMPLETION>,
--   catalog    = <CATALOG>,
--   platform   = <PLATFORM>,
-- }
--
-- EXECUTOR (adapter owns HOW an artifact is raised; kernel owns WHEN + the CAS key):
--   executor.raise_step(step_ctx) -> "raised" | "exists" | "wait" | nil,reason_code
--     step_ctx = {
--       scope, origin, blueprint, workflow_id, blueprint_digest,
--       slot, predecessor, predecessor_ref_digest, child_dedup_key,
--       generated_spec (nil; executor generates if its slot is generated),
--       facts, current, event,
--     }
--     MUST be idempotent keyed on step_ctx.child_dedup_key
--     (= materialization.child_dedup_key(origin, slot.id, predecessor_ref_digest)):
--     a repeat call returns "exists", an in-flight create returns "wait", never a double-create.
--   executor.emit_terminal(scope, origin, state, reason_code) -> (ignored)
--     state ∈ { done | blocked | error }.
--
-- COMPLETION (pure, side-effect-free child-status reader; frontier's 3rd argument):
--   completion.reader(scope) -> function(child_ref) -> status[, detail]
--     status ∈ { result_ready | fatal | recoverable | running | unknown }
--     (any other/thrown value is coerced to "unknown" by frontier).
--
-- CATALOG (blueprint provider; replaces reconcile's old dev-intake fallback):
--   catalog.load_blueprint(ctx, workflow_id) -> record   (record.blueprint is the plan)
--     ctx = { origin_proposal_id, event_ts }
--   catalog.records() -> array of built-in records   (used by the provider itself)
--
-- PLATFORM (all boundary I/O; NEVER _G):
--   platform.with_lock(lock_key, fn) -> fn()'s result
--   platform.lock_key(scope) -> string
--   platform.exec                                   -- opaque, handed to the executor's world
--   platform.wrap_pipeline_failure(name, raw)       -- optional
--   platform.discovery = {
--     list_scopes(ctx) -> array of opaque scope handles for this tick,
--     origin_of(scope) -> origin proposal id,
--     read_current(scope) -> current { state = "OPEN"|... },
--     latest_terminal(scope, current, origin) -> terminal_fact | nil,
--     latest_blueprint(scope, current, origin) -> { workflow, digest } | nil,
--     materialization_facts(scope, current, origin) -> facts array,
--     ledger_for_frontier(scope, facts) -> ledger (frontier's 2nd argument),
--     log_decision(scope, origin, from, to, outcome, reason) -> ()  (optional),
--   }
--   platform.lease = {
--     verify_claim(scope, origin) -> boolean,
--     close_done_origin(scope, origin) -> ()  (optional),
--   }
-- ===========================================================================
local digest = require("workflow.engine.digest")
local frontier = require("workflow.engine.frontier")
local materialization = require("workflow.engine.materialization")

local M = {}

M.DEFAULT_DEPT = "workflow_materialize_next"
M.DEFAULT_TICK_QUEUE = "workflow_materialization_tick"

local function tick_queue_of(seams)
  return seams.tick_queue or M.DEFAULT_TICK_QUEUE
end

local function event_queue_matches(event, queue)
  local actual = tostring(event and event.queue or "")
  return actual == queue or actual:match("%." .. queue .. "$") ~= nil
end

local function log_decision(platform, scope, origin, from_state, to_state, outcome, reason)
  local discovery = platform.discovery
  if type(discovery.log_decision) == "function" then
    discovery.log_decision(scope, origin, from_state, to_state, outcome, reason)
  end
end

-- Materialize one frontier-selected step by handing a fully-derived step_ctx to the
-- adapter's executor. The kernel derives the predecessor digest, CAS key and slot table;
-- the executor decides HOW to raise (and whether it already exists / is in flight).
function M.materialize_step(seams, scope, origin, record, blueprint_digest, facts, current, decision, event)
  local executor = seams.executor
  local slot = materialization.find_step(record.blueprint, decision.slot)
  if slot == nil then
    executor.emit_terminal(scope, origin, "error", "frontier-slot-missing")
    return "terminal"
  end

  -- Slot 1 has predecessor == nil; its predecessor_ref_digest is the EMPTY sentinel, so the
  -- CAS key is identical to the reference. Origin-self content sourcing for a generated slot 1
  -- is the executor's concern (HOW), reachable from scope/current.
  local predecessor = decision.predecessor
  local predecessor_ref_digest = materialization.predecessor_ref_digest(decision.predecessor)
  local child_dedup_key = materialization.child_dedup_key(origin, slot.id, predecessor_ref_digest)

  local step_ctx = {
    scope = scope,
    origin = origin,
    blueprint = record.blueprint,
    workflow_id = record.blueprint.id,
    blueprint_digest = blueprint_digest,
    slot = slot,
    predecessor = predecessor,
    predecessor_ref_digest = predecessor_ref_digest,
    child_dedup_key = child_dedup_key,
    generated_spec = nil,
    facts = facts,
    current = current,
    event = event,
  }

  local outcome, reason = executor.raise_step(step_ctx)
  if outcome == nil or outcome == false then
    executor.emit_terminal(scope, origin, "error", reason or "raise-step-failed")
    return "terminal"
  end
  return outcome
end

function M.process_origin(seams, scope, event)
  local platform = seams.platform
  local discovery = platform.discovery
  local executor = seams.executor
  local completion = seams.completion
  local origin = discovery.origin_of(scope)

  return platform.with_lock(platform.lock_key(scope), function()
    local current = discovery.read_current(scope)
    if tostring(current and current.state or ""):upper() ~= "OPEN" then
      log_decision(platform, scope, origin, "tick", "discover", "skip-closed", "origin is not open")
      return "skip"
    end

    -- Successful and configuration-error terminals are monotonic. A blocked terminal is a
    -- derived child verdict, so each poll recomputes it from current child facts: a child
    -- can recover after the workflow recorded child-fatal.
    local terminal_fact = discovery.latest_terminal(scope, current, origin)
    if terminal_fact ~= nil and tostring(terminal_fact.state or "") ~= "blocked" then
      if tostring(terminal_fact.state or "") == "done" and type(platform.lease.close_done_origin) == "function" then
        platform.lease.close_done_origin(scope, origin)
      end
      log_decision(platform, scope, origin, "discover", "terminal", "skip-terminal", "trusted terminal marker already exists")
      return "skip"
    end

    if not platform.lease.verify_claim(scope, origin) then
      log_decision(platform, scope, origin, "claim", "materialize", "skip-claim-lost", "origin lease is not self-held")
      return "skip"
    end

    local blueprint_fact = discovery.latest_blueprint(scope, current, origin)
    if blueprint_fact == nil then
      log_decision(platform, scope, origin, "discover", "blueprint", "skip-no-blueprint", "no trusted blueprint marker")
      return "skip"
    end

    local record = seams.catalog.load_blueprint({
      origin_proposal_id = origin,
      event_ts = event and event.ts,
    }, blueprint_fact.workflow)
    if record == nil or type(record.blueprint) ~= "table" then
      executor.emit_terminal(scope, origin, "error", "workflow-not-in-catalog")
      return "terminal"
    end

    local current_digest = digest.blueprint_digest(record.blueprint)
    if current_digest ~= blueprint_fact.digest then
      executor.emit_terminal(scope, origin, "error", "blueprint-digest-mismatch")
      return "terminal"
    end

    local facts = discovery.materialization_facts(scope, current, origin)
    local ledger = discovery.ledger_for_frontier(scope, facts)
    local decision = frontier.compute_frontier(record.blueprint, ledger, completion.reader(scope))

    if decision.action == "wait" then
      log_decision(platform, scope, origin, "frontier", "wait", "skip-wait", decision.why or "frontier-waits")
      return "wait"
    end
    if decision.action == "terminal" then
      executor.emit_terminal(scope, origin, decision.state or "error", decision.reason_code or "frontier-terminal")
      return "terminal"
    end
    if decision.action == "materialize" then
      return M.materialize_step(seams, scope, origin, record, current_digest, facts, current, decision, event)
    end

    executor.emit_terminal(scope, origin, "error", "unknown-frontier-action")
    return "terminal"
  end)
end

local function act(seams, event)
  local queue = tick_queue_of(seams)
  if not event_queue_matches(event, queue) then
    error("workflow-engine: unsupported-consumed-queue: unsupported consumed queue " .. tostring(event and event.queue or ""), 0)
  end
  local scopes = seams.platform.discovery.list_scopes({ event = event })
  for _, scope in ipairs(scopes or {}) do
    M.process_origin(seams, scope, event)
  end
end

function M.handlers(seams)
  if type(seams) ~= "table" then
    error("workflow-engine: missing-reconcile-seams: reconcile.handlers requires an injected seam table", 0)
  end
  local queue = tick_queue_of(seams)
  local wrap = seams.wrap or (seams.platform and seams.platform.wrap_pipeline_failure)
  return {
    accept = function(event)
      return event_queue_matches(event, queue)
    end,
    done = function(_event)
      return false
    end,
    act = function(event)
      return act(seams, event)
    end,
    wrap = wrap,
    name = seams.dept or M.DEFAULT_DEPT,
  }
end

function M.install(target)
  target.reconcile = M
end

return M
