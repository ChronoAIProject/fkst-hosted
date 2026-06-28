local core = require("core")
local env = require("workflow.env")
local ports_lib = require("forge.ports")
local saga = require("workflow.saga")

local spec = {
  consumes = { "integration_coverage_tick" },
  produces = { "github-proxy.github_issue_create_request" },
  stall_window = "2m",
  retry = false,
}

local busy_issue_threshold = 3
local command_timeout_seconds = 120
local github_timeout_seconds = 30
local coverage_allowlist_path = "migration/integration-edge-coverage.allowlist"

local function package_root()
  if type(debug) == "table" and type(debug.getinfo) == "function" then
    local info = debug.getinfo(1, "S")
    local source = info and info.source or ""
    if source:sub(1, 1) == "@" then
      local path = source:sub(2)
      local suffix = "/departments/produce/main.lua"
      if path:sub(-#suffix) == suffix then
        return path:sub(1, #path - #suffix)
      end
    end
  end
  return "packages/integration-coverage-producer"
end

local function checker_tool_path()
  return package_root() .. "/tools/check_repo_integration_coverage.py"
end

local function read_env_command(name)
  if name ~= "FKST_GITHUB_REPO" then
    error("integration-coverage-producer: invalid-env-name: " .. tostring(name), 0)
  end
  return 'printf %s "$' .. name .. '"'
end

local read_env = env.read_env(read_env_command, { propagate_exec_errors = true })

local function repo_from_env()
  local repo = core.trim(read_env("FKST_GITHUB_REPO") or "")
  if repo == "" then
    error("integration-coverage-producer: missing-repo: FKST_GITHUB_REPO is required", 0)
  end
  if not core.validate_repo(repo) then
    error("integration-coverage-producer: malformed-repo: FKST_GITHUB_REPO is malformed", 0)
  end
  return repo
end

local function is_tick(event)
  local queue = tostring(event and event.queue or "")
  return queue == "integration_coverage_tick" or queue == "integration-coverage-producer.integration_coverage_tick"
end

local function done(event)
  if not is_tick(event) then
    error("integration-coverage-producer: unknown-queue: " .. tostring(event and event.queue), 0)
  end
  return false
end

local function file_exists(path)
  if type(file) == "table" and type(file.exists) == "function" then
    return file.exists(path) == true
  end
  local ok = pcall(file.read, path)
  return ok == true
end

local function coverage_substrate_exists()
  return file_exists(coverage_allowlist_path)
end

local function run_checker()
  if type(exec_argv) ~= "function" then
    error("integration-coverage-producer: exec-argv-unavailable: exec_argv is required", 0)
  end
  local result = exec_argv({
    argv = { "python3", checker_tool_path(), "--json" },
    timeout = command_timeout_seconds,
  })
  if type(result) ~= "table" then
    error("integration-coverage-producer: checker-command-failed: no command result", 0)
  end
  if result.exit_code ~= 0 and core.trim(result.stdout or "") == "" then
    error("integration-coverage-producer: checker-command-failed: " .. tostring(result.stderr), 0)
  end
  return core.decode_checker_report(result.stdout or "[]")
end

local function search_issues(github, repo, query, context)
  if type(github) ~= "table" or type(github.issue_search) ~= "function" then
    error("integration-coverage-producer: issue-search-unavailable: GitHub issue_search port is required", 0)
  end
  local result = github.issue_search(repo, query, "number,title,state,author,body,labels,url", github_timeout_seconds)
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("integration-coverage-producer: issue-search-failed: " .. tostring(context), 0)
  end
  return core.decode_issue_search(result.stdout or "[]", context)
end

local function open_issues(github, repo)
  if type(github) ~= "table" or type(github.issue_list_cli) ~= "function" then
    error("integration-coverage-producer: issue-list-unavailable: GitHub issue_list_cli port is required", 0)
  end
  local result = github.issue_list_cli(
    repo,
    core.open_issue_state(),
    core.open_issue_limit(),
    core.open_issue_fields(),
    github_timeout_seconds
  )
  if type(result) ~= "table" or result.exit_code ~= 0 then
    error("integration-coverage-producer: issue-list-failed: devloop idle gate", 0)
  end
  return core.decode_open_issue_list(result.stdout or "[]")
end

local function board_is_busy(github, repo)
  local issues = open_issues(github, repo)
  local count = core.devloop_issue_count(issues)
  if count > busy_issue_threshold then
    log.info("integration-coverage-producer: skip busy board open_devloop_issues=" .. tostring(count))
    return true
  end
  return false
end

local function existing_open_issue_for_edge(github, repo, edge)
  local marker = core.coverage_marker(edge.edge_id)
  local issues = search_issues(github, repo, core.issue_search_query(edge.edge_id), "coverage issue search")
  return core.has_open_issue_with_marker(issues, marker)
end

local function raise_first_eligible(github, repo, edges)
  for _, edge in ipairs(edges) do
    if existing_open_issue_for_edge(github, repo, edge) then
      log.info("integration-coverage-producer: skip existing coverage issue edge_id=" .. tostring(edge.edge_id))
    else
      local request = core.issue_create_request(repo, edge)
      log.info("integration-coverage-producer: file coverage issue edge_id=" .. tostring(edge.edge_id))
      raise("github-proxy.github_issue_create_request", request)
      return
    end
  end
  log.info("integration-coverage-producer: skip all uncovered edges already have open issues")
end

local function make_department(ports)
  ports = ports or {}
  local function act(event)
    if not is_tick(event) then
      error("integration-coverage-producer: unknown-queue: " .. tostring(event and event.queue), 0)
    end
    if not coverage_substrate_exists() then
      log.info("integration-coverage-producer: skip not applicable here missing_allowlist=" .. coverage_allowlist_path)
      return
    end
    local report = run_checker()
    local edges = core.uncovered_allowlisted_edges(report)
    if #edges == 0 then
      log.info("integration-coverage-producer: skip no uncovered allowlisted edges")
      return
    end
    local repo = repo_from_env()
    if board_is_busy(ports.github, repo) then
      return
    end
    raise_first_eligible(ports.github, repo, edges)
  end

  local department = saga.department(spec, {
    done = done,
    act = act,
    name = "produce",
  })
  department.ports = ports
  return department
end

return ports_lib.install(make_department)
