local codex_lib = require("workflow.codex")
local env_port = require("departments.health_report.env_port")
local error_facts = require("contract.error_facts")
local health = require("health")
local ports_lib = require("forge.ports")
local probes = require("departments.health_report.probes")
local prompt = require("departments.health_report.prompt")
local report = require("report")
local saga = require("workflow.saga")
local strings = require("contract.strings")
local writer = require("departments.health_report.writer")

local spec = {
  consumes = { "health_tick" },
  produces = { "health_report_written" },
  -- Broadcast: any number of sibling packages may subscribe to "a health report was
  -- written", exactly as idle-detector broadcasts system_idle. Nothing does today,
  -- and the engine warns-and-drops an unsubscribed fanout queue -- the same shape
  -- github-comment-effect's github_comment_written already ships with.
  fanout = { "health_report_written" },
  stall_window = "10m",
  -- A missed window is not worth replaying: the next tick is ten minutes away and a
  -- stale report is worse than an absent one.
  retry = false,
}

-- FAILURE POSTURE. This department ships in the default manifest, so every defect is
-- a fleet-wide defect. Every operational failure below -- a refused probe, a codex
-- that never answers, a filesystem that will not take the write -- is caught, logged,
-- and degraded. Nothing outside `<FKST_RUNTIME_ROOT>/health` is touched. Only an
-- unroutable event errors, because that is a wiring bug rather than a bad window.
--
-- Probe failures are recorded as FIXED reason strings, never as the underlying error
-- text. The evidence goes to a codex and into a report that leaves the pod, and a
-- fixed vocabulary is what makes it impossible for an error message to carry a
-- credential there; the real error is logged instead, where the collector redacts it.
local probe_failed = {
  observe = "observe probe unavailable",
  codex = "codex run status unavailable",
  repository = "commit count unavailable",
  work_items = "work item query unavailable",
}

local context_leaf = ".fkst-health-context.md"
local issue_query_timeout_seconds = 30
local issue_query_fields = "number,state"

-- Accept BOTH the bare and the namespaced production queue name. The engine delivers
-- "fkst-health.health_tick"; comparing event.queue against the bare literal alone is
-- what the G-NAMESPACED-QUEUE ratchet exists to catch.
local function is_health_tick(event)
  local queue = tostring(event and event.queue or "")
  return queue == "health_tick" or queue == "fkst-health.health_tick"
end

local function note(tag, event, why)
  local fields = error_facts.error_fact_fields(
    "degraded-signal",
    type(event) == "table" and event.queue or nil,
    "health_report",
    why,
    { source_ref = error_facts.event_source_ref(event) }
  )
  table.insert(fields, "WHY=" .. error_facts.one_line(why))
  log.warn("fkst-health dept=health_report tag=" .. tostring(tag) .. " " .. table.concat(fields, " "))
end

local function wrap_pipeline_failure(dept, fn)
  return function(event)
    local ok, result = pcall(fn, event)
    if ok then
      return result
    end
    local fields = error_facts.error_fact_fields(
      "caught-failure",
      type(event) == "table" and event.queue or nil,
      dept,
      result,
      { source_ref = error_facts.event_source_ref(event) }
    )
    table.insert(fields, "error=" .. error_facts.one_line(result))
    log["error"]("fkst-health dept=" .. dept .. " tag=FAILURE " .. table.concat(fields, " "))
    error(("fkst-health: caught-failure: " .. tostring(result)), 0)
  end
end

-- The durable root is NEVER passed as a string. fkst.observe resolves FKST_DURABLE_ROOT
-- itself, in process, exactly as the session was launched with it. That matters: the
-- engine derives its live-observe socket path from a hash of that string AS GIVEN, so
-- a single appended slash silently degrades every call into a redb-lock error against
-- the running supervisor. Not touching the value at all is the only way to be sure.
local function production_observe()
  if type(fkst) ~= "table" or type(fkst.observe) ~= "function" then
    error("fkst-health: observe-unavailable: fkst.observe is required", 0)
  end
  return fkst.observe()
end

local function production_codex_runs()
  if type(fkst) ~= "table" or type(fkst.codex_runs) ~= "function" then
    error("fkst-health: codex-runs-unavailable: fkst.codex_runs is required", 0)
  end
  return fkst.codex_runs()
end

local function production_commit_count(window_seconds)
  if type(git_log_count) ~= "function" then
    error("fkst-health: git-log-unavailable: git_log_count is required", 0)
  end
  -- An empty --grep matches every commit, so this is "commits in the window".
  return git_log_count("", tostring(window_seconds) .. " seconds ago")
end

local function production_run_codex(opts)
  if type(spawn_codex_sync) ~= "function" then
    error("fkst-health: codex-unavailable: spawn_codex_sync is required", 0)
  end
  return spawn_codex_sync(opts)
end

local function production_now()
  if type(now) ~= "function" then
    error("fkst-health: now-unavailable: now primitive is required", 0)
  end
  return now()
end

local function optional_global(name)
  local value = _G[name]
  if type(value) == "function" then
    return value
  end
  return nil
end

local function health_done(event)
  if not is_health_tick(event) then
    error("fkst-health: unknown-queue: " .. tostring(event and event.queue), 0)
  end
  return false
end

local function decode_issues(stdout)
  local ok, decoded = pcall(json.decode, tostring(stdout or "[]"))
  if not ok or type(decoded) ~= "table" then
    return nil
  end
  return decoded
end

local function make_department(ports)
  ports = ports or {}
  local read_env = ports.read_env or env_port.read
  local observe = ports.observe or production_observe
  local codex_runs = ports.codex_runs or production_codex_runs
  local commit_count = ports.commit_count or production_commit_count
  local run_codex = ports.run_codex or production_run_codex
  local clock = ports.now or production_now
  local files = writer.new(ports)

  local function cache_reader()
    return ports.cache_get or optional_global("cache_get")
  end

  local function cache_writer()
    return ports.cache_set or optional_global("cache_set")
  end

  local function env(name)
    local ok, value = pcall(read_env, name)
    if not ok then
      return ""
    end
    return strings.trim(value or "")
  end

  -- Each probe is fetched under its own pcall and folded into its own signal. A probe
  -- that fails degrades exactly that signal; the tick always continues.
  local function collect(event, memory)
    local observations, snapshot = {}, {}

    local ok_observe, facts = pcall(observe)
    if ok_observe then
      local deliveries, faults, seen = probes.from_observe(facts, memory)
      observations.deliveries = deliveries
      observations.faults = faults
      for key, value in pairs(seen) do
        snapshot[key] = value
      end
    else
      note("SKIP", event, "observe probe failed: " .. error_facts.one_line(facts))
      observations.deliveries = probes.unreadable(probe_failed.observe)
      observations.faults = probes.unreadable(probe_failed.observe)
    end

    local ok_runs, status = pcall(codex_runs)
    if ok_runs then
      local signal, seen = probes.from_codex_runs(status, memory)
      observations.codex = signal
      for key, value in pairs(seen) do
        snapshot[key] = value
      end
    else
      note("SKIP", event, "codex run status probe failed: " .. error_facts.one_line(status))
      observations.codex = probes.unreadable(probe_failed.codex)
    end

    local ok_commits, commits = pcall(commit_count, health.expected_interval_seconds)
    if ok_commits then
      observations.repository = probes.from_commit_count(commits)
    else
      note("SKIP", event, "commit count probe failed: " .. error_facts.one_line(commits))
      observations.repository = probes.unreadable(probe_failed.repository)
    end

    observations.work_items = probes.unreadable(probe_failed.work_items)
    local repo, label = env("FKST_GITHUB_REPO"), env("FKST_SESSION_WORK_LABEL")
    -- Resolving the handle is itself fallible and MUST be guarded. When the
    -- production wiring cannot build a trusted-author policy, forge.ports makes
    -- ports.github a poison object whose metatable raises on any field access -- so
    -- merely reading `github.issue_search` to test it raises, outside every pcall,
    -- and kills the whole tick. This package ships in the default manifest, so a
    -- tick that dies instead of degrading one signal is a fleet-wide outage.
    local ok_handle, issue_search = pcall(function()
      local handle = ports.github
      if type(handle) ~= "table" then
        return nil
      end
      local fn = handle.issue_search
      return type(fn) == "function" and fn or nil
    end)
    if not ok_handle then
      note("SKIP", event, "github port unavailable: " .. error_facts.one_line(issue_search))
      issue_search = nil
    end
    if repo ~= "" and label ~= "" and type(issue_search) == "function" then
      local ok_issues, result = pcall(
        issue_search,
        repo,
        'is:open label:"' .. label .. '"',
        issue_query_fields,
        issue_query_timeout_seconds
      )
      local issues = nil
      if ok_issues and type(result) == "table" and tonumber(result.exit_code) == 0 then
        issues = decode_issues(result.stdout)
      end
      if issues == nil then
        note("SKIP", event, "work item probe failed for label " .. label)
      else
        local signal, seen = probes.from_issue_list(issues, memory)
        observations.work_items = signal
        for key, value in pairs(seen) do
          snapshot[key] = value
        end
      end
    end

    return observations, snapshot
  end

  -- The judge NARRATES the verdict; it never changes it. Anything it returns becomes
  -- the report body and nothing else, so a reply claiming a different status cannot
  -- reach the emitted `status`.
  local function narrate(event, verdict, directory, session_id, fault)
    local context_path = directory .. "/" .. context_leaf
    if not files.write_context(context_path, prompt.context(verdict, session_id, fault)) then
      note("SKIP", event, "evidence context could not be written")
      return prompt.fallback_body(verdict, "evidence context unavailable", fault)
    end
    -- judgment_codex_opts already sets sandbox = "read-only" and carries no
    -- role/proposal_id/dedup_key, which is what keeps a raw spawn compliant with the
    -- live-run-dispatch ratchet. The worktree is "." because a read-only-sandbox
    -- codex refuses to start outside a git repository; it reads its evidence from the
    -- absolute context path above.
    local opts = codex_lib.judgment_codex_opts(prompt.build(verdict, context_path, fault), ".")
    opts.timeout = prompt.timeout_seconds

    local ok_codex, result = pcall(run_codex, opts)
    local why = nil
    if not ok_codex then
      why = "codex spawn failed"
    elseif type(result) ~= "table" then
      why = "codex returned no result"
    elseif tonumber(result.exit_code) == 124 then
      why = "codex timed out"
    elseif tonumber(result.exit_code) ~= 0 then
      why = "codex exited non-zero"
    else
      local body = prompt.body_from_reply(result.stdout)
      if body ~= nil then
        return body
      end
      why = "codex produced no narrative"
    end
    note("SKIP", event, "narrative unavailable: " .. why)
    return prompt.fallback_body(verdict, why, fault)
  end

  local function act_health(event)
    if not is_health_tick(event) then
      error("fkst-health: unknown-queue: " .. tostring(event and event.queue), 0)
    end

    local runtime_root = env("FKST_RUNTIME_ROOT")
    local session_id = env("FKST_SESSION_ID")
    if runtime_root == "" or session_id == "" then
      -- Without a scratch root there is nowhere to write, and without a session id the
      -- control plane's parser rejects the document outright. Either way the honest
      -- outcome is no report, loudly.
      note("SKIP", event, "FKST_RUNTIME_ROOT and FKST_SESSION_ID are both required")
      return
    end
    local namespace = env("FKST_WORK_LABEL_NAMESPACE")

    local memory = probes.recall(cache_reader())
    local observations, snapshot = collect(event, memory)
    -- The count INCLUDES this window, so a first quiet window reports 1 and stall
    -- confidence only reaches `high` once a second consecutive window agrees.
    observations.window = { consecutive_no_progress = (memory.quiet or 0) + 1 }

    local verdict = health.decide(observations)
    local generated_at = clock()
    local directory = files.directory(runtime_root)
    local ok_directory, directory_why = files.ensure(directory)
    if not ok_directory then
      note("SKIP", event, "health directory unavailable: " .. tostring(directory_why))
      return
    end

    -- The observe snapshot truncates its error excerpt before the actual cause, so
    -- go and read the failing department's own log for the terminal error. Failure
    -- here is fine: the report then names the log instead of quoting it.
    local fault = health.fault_detail(observations)
    if fault ~= nil then
      local ok_reason, reason, log_path = pcall(files.terminal_error, runtime_root, fault.dept)
      if ok_reason and reason ~= nil then
        fault.reason, fault.log_path = reason, log_path
      elseif ok_reason then
        fault.log_path = log_path
      end
    end

    local body = narrate(event, verdict, directory, session_id, fault)
    local text = report.render({
      session_id = session_id,
      namespace = namespace ~= "" and namespace or nil,
      generated_at = generated_at,
      window_start = generated_at - health.expected_interval_seconds,
      status = verdict.status,
      headline = verdict.headline,
      confidence = verdict.confidence,
      evidence = verdict.evidence,
      work_items = verdict.work_items,
      body = body,
    }, health.expected_interval_seconds)

    local name = report.filename(namespace, session_id, generated_at)
    local published, publish_why = files.publish(directory, name, text)
    if published then
      log.info(
        "fkst-health dept=health_report tag=REPORT status="
          .. verdict.status
          .. " confidence="
          .. tostring(verdict.confidence)
          .. " file="
          .. name
      )
      raise("health_report_written", {
        schema = "fkst-health.report-written.v1",
        session_id = session_id,
        status = verdict.status,
        confidence = verdict.confidence,
        file = name,
        source_ref = { kind = "cron", ref = "fkst-health/health_poll/" .. report.stamp(generated_at) },
      })
    else
      note("SKIP", event, "report could not be published: " .. tostring(publish_why))
    end

    files.prune(directory)
    snapshot.quiet = probes.next_quiet_windows(memory, verdict.progressed)
    probes.remember(cache_writer(), snapshot)
  end

  local department = saga.department(spec, {
    done = health_done,
    act = act_health,
    wrap = wrap_pipeline_failure,
    name = "health_report",
  })
  department.ports = ports
  return department
end

-- Production wiring. WITHOUT the author-policy options, forge.ports installs a
-- POISON OBJECT as ports.github (ports.lua: a metatable whose __index raises on any
-- field access), so the work-item probe cannot run at all. Mirrors
-- idle-detector/departments/idle_gate/main.lua, which passes the same shape.
local github_author_policy_env = {
  bot_login_env = "FKST_GITHUB_BOT_LOGIN",
  extra_login_envs = { "FKST_GITHUB_AUTHORIZED_LOGINS" },
}

return ports_lib.install(
  make_department,
  ports_lib.github_author_options(env_port.read, "fkst-health", github_author_policy_env)
)
