-- workflow.engine.departments
--
-- make_departments(config) -> { select, materialize_next, dead_letter }
--
-- Returns LAZY per-department closures, NOT eagerly-built departments. Laziness is
-- MANDATORY: saga.department mutates _G.pipeline, so building all three eagerly would make
-- the last clobber the others. Each adapter's departments/*/main.lua is a ~3-line wrapper
-- that calls exactly one of these closures:
--
--   local engine = require("workflow.engine")
--   return engine.make_departments(bindings).materialize_next()
--
-- ===========================================================================
-- config = {
--   -- the four seams (see workflow.engine.reconcile for full shapes):
--   namespace  = "<marker-token>",     -- e.g. "fkst:workflow-security"
--   executor   = <EXECUTOR>,           -- { raise_step, emit_terminal }
--   completion = <COMPLETION>,         -- { reader }
--   catalog    = <CATALOG>,            -- { load_blueprint, records }
--   platform   = <PLATFORM>,           -- { with_lock, lock_key, exec, discovery, lease, ... }
--
--   -- department wiring (queue names + spec overrides; adapter-owned, kernel-agnostic):
--   package    = "<adapter-package>",  -- dead-letter log tag
--   tick_queue = "<queue>",            -- reconcile tick queue (default below)
--   dept       = "<name>",             -- reconcile department name (optional)
--   wrap       = function(name, raw) ... end,   -- optional pipeline-failure wrapper
--
--   materialize_next = {               -- optional spec overrides for the reconcile dept
--     consumes = { ... }, published_seam = { ... }, produces = { ... }, stall_window = "2m",
--   },
--   intake = {                         -- the adapter's OWN intake/select department
--     consumes = { ... }, produces = { ... }, stall_window = "2m",
--     handlers = function() return <saga handler set> end,   -- REQUIRED to build select()
--   },
--   dead_letter = {                    -- optional dead-letter spec overrides
--     consumes = { "dead_letter" }, produces = { ... }, stall_window = "2m", package = "...",
--   },
-- }
-- ===========================================================================
local dead_letter = require("workflow.dead_letter")
local reconcile = require("workflow.engine.reconcile")
local saga = require("workflow.saga")

local M = {}

local DEFAULT_STALL_WINDOW = "2m"

local function seams_from(config)
  return {
    namespace = config.namespace,
    executor = config.executor,
    completion = config.completion,
    catalog = config.catalog,
    platform = config.platform,
    tick_queue = config.tick_queue,
    dept = config.dept,
    wrap = config.wrap or (config.platform and config.platform.wrap_pipeline_failure),
  }
end

function M.make_departments(config)
  config = config or {}
  local seams = seams_from(config)
  local default_tick = config.tick_queue or reconcile.DEFAULT_TICK_QUEUE

  -- The reconcile-driven department: one tick materializes the next frontier step.
  local function materialize_next()
    local overrides = config.materialize_next or {}
    local consumes = overrides.consumes or { default_tick }
    local spec = {
      consumes = consumes,
      published_seam = overrides.published_seam or consumes,
      produces = overrides.produces or {},
      stall_window = overrides.stall_window or DEFAULT_STALL_WINDOW,
    }
    return saga.department(spec, reconcile.handlers(seams))
  end

  -- The adapter's OWN intake seat. The kernel keeps the spec generic; the adapter supplies
  -- its own consumes/produces queues and its own handler factory (never the dev candidate
  -- seam unless the adapter is workflow-develop).
  local function select()
    local intake = config.intake or config.select
    if type(intake) ~= "table" or type(intake.handlers) ~= "function" then
      error("workflow-engine: missing-intake-binding: make_departments.select needs config.intake.handlers", 0)
    end
    local spec = {
      consumes = intake.consumes,
      published_seam = intake.published_seam,
      produces = intake.produces or {},
      stall_window = intake.stall_window or DEFAULT_STALL_WINDOW,
    }
    return saga.department(spec, intake.handlers())
  end

  local function dead_letter_department()
    local overrides = config.dead_letter or {}
    local spec = {
      consumes = overrides.consumes or { "dead_letter" },
      produces = overrides.produces or {},
      stall_window = overrides.stall_window or DEFAULT_STALL_WINDOW,
    }
    return saga.department(spec, dead_letter.handlers({
      package = overrides.package or config.package,
      wrap = seams.wrap,
    }))
  end

  return {
    select = select,
    materialize_next = materialize_next,
    dead_letter = dead_letter_department,
  }
end

function M.install(target)
  target.make_departments = M.make_departments
  return M
end

return M
