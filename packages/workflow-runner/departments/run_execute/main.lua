local runner = require("runner")
local env = require("workflow.env")
local codex = require("workflow.codex")
local error_facts = require("contract.error_facts")
local claim_identity = require("forge.github.claim_identity")
local ports_lib = require("forge.ports")
local saga = require("workflow.saga")

-- Execute one scheduled run's steps, in declared order.
--
-- The linear executor is deliberate. `libraries/workflow/engine` materializes
-- every step as a child issue worked into a pull request, and its content kinds
-- are closed to {static, generated}. The workloads this package exists for
-- produce no code and no PR, so that engine is the wrong substrate: it would file
-- an issue per step for work that has nothing to review.
--
-- A failed step FAILS the run and later steps do not execute. Half a pipeline is
-- worse than none: a sourcing workflow whose scrape failed and whose publish ran
-- anyway would publish an empty result over a good one.
local spec = {
  consumes = { "scheduled_run_tick" },
  produces = { "scheduled_run_result" },
  stall_window = "6h",
  -- A failed run waits for its next slot rather than re-running inside the same
  -- boot: a retry here would double the load of whatever just failed, and the
  -- control-plane clock already owns "when to try again".
  retry = false,
}

-- The run's identity comes from the RUN ISSUE, and the session's identity from
-- platform-owned env. Neither is ever sourced from FKST_SESSION_PACKAGE_ENV_JSON
-- — that is the author-written `### Package Env` blob, and a trigger author must
-- not be able to forge which workflow runs or what it executes.
local allowed_env = {
  FKST_GITHUB_REPO = true,
  FKST_GITHUB_BOT_LOGIN = true,
  FKST_SESSION_CREATOR = true,
}

local function read_env_command(name)
  if not allowed_env[name] then
    error("workflow-runner: env-name-denied: " .. tostring(name), 0)
  end
  return 'printf %s "$' .. name .. '"'
end

local production_read_env = env.read_env(read_env_command, { propagate_exec_errors = true })
local github_author_policy_env = { bot_login_env = "FKST_GITHUB_BOT_LOGIN" }

local function production_now()
  if type(now) ~= "function" then
    error("workflow-runner: now-unavailable: now primitive is required", 0)
  end
  return now()
end

local function iso(seconds)
  return os.date("!%Y-%m-%dT%H:%M:%SZ", math.floor(seconds))
end

local function is_run_tick(event)
  local queue = tostring(event and event.queue or "")
  return queue == "scheduled_run_tick" or queue == "workflow-runner.scheduled_run_tick"
end

--- Read the definition file out of the session workspace.
local function read_definition(run_exec, path)
  local ok, result = pcall(run_exec, { cmd = ("cat -- %q"):format(path), timeout = 30 })
  if not ok or type(result) ~= "table" or result.exit_code ~= 0 then
    return nil, ("cannot read %s"):format(path)
  end
  return result.stdout, nil
end

--- Execute one deterministic step.
local function run_step(run_exec, step)
  local started = production_now()
  -- argv elements are shell-quoted individually, so a substituted argument
  -- containing `;` or a quote stays one argument rather than becoming syntax.
  local quoted = {}
  for index, element in ipairs(step.argv) do
    quoted[index] = ("%q"):format(element)
  end
  local ok, result = pcall(run_exec, {
    cmd = table.concat(quoted, " "),
    timeout = step.timeout_secs,
  })
  local elapsed = production_now() - started
  if not ok or type(result) ~= "table" then
    return {
      status = "failed",
      duration_s = elapsed,
      detail = ("step %s could not start: %s"):format(step.id, error_facts.one_line(result)),
      tail = "",
    }
  end
  local tail = runner.truncate_tail((result.stdout or "") .. (result.stderr or ""))
  if result.exit_code ~= 0 then
    return {
      status = "failed",
      duration_s = elapsed,
      detail = ("step %s exited %s"):format(step.id, tostring(result.exit_code)),
      tail = tail,
    }
  end
  return { status = "ok", duration_s = elapsed, tail = tail }
end

--- Execute one agentic step, honouring the session's own engine configuration.
local function task_step(run_codex, step)
  local started = production_now()
  local ok, result = pcall(run_codex, codex.unrestricted_codex_opts(step.prompt, nil))
  local elapsed = production_now() - started
  if not ok or type(result) ~= "table" or tostring(result.status) ~= "completed" then
    return {
      status = "failed",
      duration_s = elapsed,
      detail = ("task step %s did not complete"):format(step.id),
      tail = "",
    }
  end
  return { status = "ok", duration_s = elapsed, tail = "" }
end

--- Walk the steps, stopping at the first failure and marking the rest skipped.
local function execute(steps, arguments, run_exec, run_codex)
  local outcomes, tail, detail, status = {}, "", nil, "ok"
  for _, step in ipairs(steps) do
    if status ~= "ok" then
      outcomes[#outcomes + 1] = { index = step.index, id = step.id, status = "skipped" }
    else
      local resolved, resolve_error = runner.resolve_step(step, arguments)
      if resolve_error ~= nil then
        status, detail = "failed", resolve_error
        outcomes[#outcomes + 1] = { index = step.index, id = step.id, status = "failed" }
      else
        local outcome = resolved.kind == "run" and run_step(run_exec, resolved)
          or task_step(run_codex, resolved)
        outcomes[#outcomes + 1] = {
          index = step.index,
          id = step.id,
          status = outcome.status,
          duration_s = outcome.duration_s,
        }
        if outcome.tail ~= nil and outcome.tail ~= "" then
          tail = outcome.tail
        end
        if outcome.status ~= "ok" then
          status, detail = "failed", outcome.detail
        end
      end
    end
  end
  return status, outcomes, tail, detail
end

local function make_department(ports)
  ports = ports or {}
  local github = ports.github
  local read_env = ports.read_env or production_read_env
  local run_exec = ports.exec or function(request)
    return exec_sync(request)
  end
  local run_codex = ports.codex or function(opts)
    return codex_run(opts)
  end

  local function act_run(event)
    if not is_run_tick(event) then
      error("workflow-runner: unknown-queue: " .. tostring(event and event.queue), 0)
    end

    local identity, identity_error = claim_identity.read(read_env)
    if identity_error ~= nil then
      -- No verified repo scope: there is nothing safe to read or report against.
      return
    end
    local creator = read_env("FKST_SESSION_CREATOR")
    local listed = github.issue_list_intake(identity.repo, 50, 30)
    local issues = type(listed) == "table" and (listed.issues or listed.value or listed) or {}
    local issue, dispatch = runner.select_run_issue(issues, creator)
    if issue == nil then
      -- Not a scheduled run: an ordinary session boot must be a clean no-op.
      return
    end

    local started = production_now()
    local function fail(detail)
      raise("scheduled_run_result", {
        repo = identity.repo,
        schedule_issue = dispatch.schedule_issue,
        run_issue = issue.number,
        record = {
          slot = dispatch.slot,
          manual = dispatch.manual,
          status = "failed",
          started = iso(started),
          ended = iso(production_now()),
          issue = issue.number,
          detail = detail,
          steps = {},
        },
      })
    end

    local path, path_error = runner.definition_path(dispatch.workflow_id)
    if path_error ~= nil then
      return fail(path_error)
    end
    local text, read_error = read_definition(run_exec, path)
    if read_error ~= nil then
      return fail(read_error)
    end
    local decoded, decode_error = require("runner.toml").decode(text)
    if decode_error ~= nil then
      return fail(("%s: %s"):format(path, decode_error))
    end
    local steps, definition_error = runner.validate_definition(decoded)
    if definition_error ~= nil then
      return fail(("%s: %s"):format(path, definition_error))
    end

    local status, outcomes, tail, detail = execute(steps, dispatch.arguments, run_exec, run_codex)
    raise("scheduled_run_result", {
      repo = identity.repo,
      schedule_issue = dispatch.schedule_issue,
      run_issue = issue.number,
      record = {
        slot = dispatch.slot,
        manual = dispatch.manual,
        status = status,
        started = iso(started),
        ended = iso(production_now()),
        issue = issue.number,
        detail = detail,
        steps = outcomes,
        tail = tail,
      },
    })
  end

  local department = saga.department(spec, {
    done = function(event)
      if not is_run_tick(event) then
        error("workflow-runner: unknown-queue: " .. tostring(event and event.queue), 0)
      end
      return false
    end,
    act = act_run,
    name = "run_execute",
  })
  department.ports = ports
  return department
end

return ports_lib.install(
  make_department,
  ports_lib.github_author_options(production_read_env, "workflow-runner", github_author_policy_env)
)
