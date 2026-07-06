local env = require("workflow.env")
local error_facts = require("contract.error_facts")
local ports_lib = require("forge.ports")
local ref_detect_caps = require("ref_detect_caps")
local saga = require("workflow.saga")

local spec = {
  consumes = { "git_ref_poll_tick" },
  produces = { "git_ref_changed" },
  fanout = { "git_ref_changed" },
  stall_window = "30s",
}

local allowed_env = {
  FKST_GIT_WATCH_REFS = true,
}

local function read_env_command(name)
  if not allowed_env[name] then
    error("git-branch-detector: env-name-denied: " .. tostring(name), 0)
  end
  return 'printf %s "$' .. name .. '"'
end

local production_read_env = env.read_env(read_env_command, { propagate_exec_errors = true })

local function production_now()
  if type(now) ~= "function" then
    error("git-branch-detector: now-unavailable: now primitive is required", 0)
  end
  return now()
end

local function is_git_ref_poll_tick(event)
  local queue = tostring(event and event.queue or "")
  return queue == "git_ref_poll_tick" or queue == "git-branch-detector.git_ref_poll_tick"
end

local function log_skip(reason, event)
  local caps = ref_detect_caps
  log.warn(caps.skip_fact("ref_detect", event, reason, true))
end

local function wrap_pipeline_failure(dept, fn)
  return function(event)
    local ok, result = pcall(fn, event)
    if ok then
      return result
    end
    local fields = error_facts.error_fact_fields("caught-failure", type(event) == "table" and event.queue or nil, dept, result, {
      source_ref = error_facts.event_source_ref(event),
    })
    table.insert(fields, "error=" .. error_facts.one_line(result))
    log["error"]("git-branch-detector dept=" .. dept .. " tag=FAILURE " .. table.concat(fields, " "))
    error("git-branch-detector: caught-failure: " .. tostring(result), 0)
  end
end

local function ref_detect_done(event)
  if not is_git_ref_poll_tick(event) then
    error("git-branch-detector: unknown-queue: " .. tostring(event and event.queue), 0)
  end
  return false
end

local function make_department(ports)
  ports = ports or {}
  local caps = ports.caps or ref_detect_caps
  local git = ports.git
  local read_env = ports.read_env or production_read_env
  local now_value = ports.now or production_now

  local function act_ref_detect(event)
    if not is_git_ref_poll_tick(event) then
      error("git-branch-detector: unknown-queue: " .. tostring(event and event.queue), 0)
    end

    local targets = caps.parse_watch_refs(read_env("FKST_GIT_WATCH_REFS"))
    if #targets == 0 then
      log_skip("missing FKST_GIT_WATCH_REFS", event)
      return
    end

    local observed_at = caps.observed_at(now_value())
    for _, target in ipairs(targets) do
      local sha = caps.lookup_remote_branch_sha(git, target)
      if sha == nil then
        log_skip("remote branch not found: " .. target.ref, event)
      else
        raise("git_ref_changed", caps.git_ref_changed_payload(target, sha, observed_at))
      end
    end
  end

  local department = saga.department(spec, {
    done = ref_detect_done,
    act = act_ref_detect,
    wrap = wrap_pipeline_failure,
    name = "ref_detect",
  })
  department.ports = ports
  return department
end

return ports_lib.install(make_department)
