local H = {}

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\"'\"'") .. "'"
end

local function read_command(command)
  local handle = assert(io.popen(command .. " 2>&1"))
  local output = handle:read("*a")
  local ok = handle:close()
  if ok == false or ok == nil then
    error("archaudit fire_raiser fixture command failed: " .. tostring(command) .. "\n" .. tostring(output))
  end
  return output
end

local function run_command(command)
  read_command(command)
end

local function repo_root()
  return (read_command("pwd"):gsub("%s+$", ""))
end

local function temp_root(name)
  return (read_command("mktemp -d " .. shell_quote("/tmp/fkst-archaudit-fire-raiser-" .. tostring(name) .. ".XXXXXX")):gsub("%s+$", ""))
end

local function copy_dir(src, dst)
  run_command("mkdir -p " .. shell_quote(dst))
  run_command("cp -R " .. shell_quote(src) .. "/. " .. shell_quote(dst) .. "/")
end

local function write_file(path, body)
  file.write(path, body)
end

function H.setup_workspace(name, child_test)
  local root = temp_root(name)
  local source = repo_root()
  write_file(root .. "/fkst.workspace.toml", '[workspace]\nunits = ["packages/*", "libraries/*"]\n')
  for _, lib in ipairs({ "contract", "workflow", "testkit", "forge" }) do
    copy_dir(source .. "/libraries/" .. lib, root .. "/libraries/" .. lib)
  end
  copy_dir(source .. "/packages/archaudit", root .. "/packages/archaudit")
  run_command("rm -rf " .. shell_quote(root .. "/packages/archaudit/tests"))
  copy_dir(source .. "/packages/idle-detector", root .. "/packages/idle-detector")
  run_command("rm -rf " .. shell_quote(root .. "/packages/idle-detector/tests"))
  run_command("mkdir -p " .. shell_quote(root .. "/packages/github-proxy/departments/github_issue_create"))
  write_file(root .. "/packages/github-proxy/fkst.toml", [[
kind = "package"
name = "github-proxy"

[code]
root = "."
]])
  write_file(root .. "/packages/github-proxy/departments/github_issue_create/main.lua", [[
local M = {}
M.spec = {
  consumes = { "github_issue_create_request" },
  published_seam = { "github_issue_create_request" },
  stall_window = "30s",
}
function M.pipeline(_event)
end
return M
]])
  run_command("mkdir -p " .. shell_quote(root .. "/packages/archaudit/tests"))
  write_file(root .. "/packages/archaudit/tests/fire_raiser_child_test.lua", child_test)
  return root
end

local function framework_bin()
  local bin = os.getenv("BIN") or "/Users/auric/fkst-substrate/target/debug/fkst-framework"
  if bin == "" then
    error("archaudit fire_raiser fixture requires BIN")
  end
  return bin
end

function H.run_child(root)
  local bin = framework_bin()
  local command = table.concat({
    "BIN=" .. shell_quote(bin),
    "FKST_RUNTIME_ROOT=" .. shell_quote(root .. "/runtime"),
    "FKST_DURABLE_ROOT=" .. shell_quote(root .. "/durable"),
    shell_quote(bin),
    "test",
    "--project-root",
    shell_quote(root .. "/packages/archaudit"),
    "--package-root",
    shell_quote(root .. "/packages/archaudit"),
    "--package-root",
    shell_quote(root .. "/packages/idle-detector"),
    "--package-root",
    shell_quote(root .. "/packages/github-proxy"),
  }, " ")
  return read_command(command)
end

function H.fire_raiser_child(body)
  return [[
local t = fkst.test

local function mock_env(repo, max_issues)
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = repo or "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$ARCHAUDIT_MAX_ISSUES_PER_IDLE"', { stdout = max_issues or "3", stderr = "", exit_code = 0 })
end

local function observe_facts(generated_at_ms, queue)
  return {
    schema_version = 1,
    generated_at_ms = generated_at_ms or 1781830860000,
    source = {
      durable_root = "/tmp/fkst-durable",
      database = "/tmp/fkst-durable/delivery.redb",
      read_semantics = "single read transaction",
      history_semantics = "delivery queue snapshot only",
    },
    limits = { max_deliveries = 500, max_dead_letters = 500 },
    truncated = { deliveries = false, dead_letters = false },
    queues = {
      queue,
    },
    deliveries = json.decode("[]"),
    dead_letters = json.decode("[]"),
  }
end

local function mock_idle_observe_at(generated_at_ms)
  t.mock_observe(observe_facts(generated_at_ms, {
    queue = "proposal",
    depth = 0,
    pending = 0,
    in_flight = 0,
    retrying = 0,
    oldest_pending_age_ms = nil,
  }))
end

local function mock_idle_observe()
  mock_idle_observe_at(1781830860000)
end

local function mock_busy_observe_at(generated_at_ms)
  t.mock_observe(observe_facts(generated_at_ms, {
    queue = "proposal",
    depth = 1,
    pending = 1,
    in_flight = 0,
    retrying = 0,
    oldest_pending_age_ms = 1000,
  }))
end

local function mock_busy_observe()
  mock_busy_observe_at(1781830860000)
end

local function mock_production_github(search_stdout, label_stdout)
  t.mock_command("gh issue list --repo owner/repo --state all --limit 100 --search fkst:archaudit:audit-run:v1 --json 'number,title,state,author,body,url,createdAt,updatedAt'", {
    stdout = search_stdout or "[]",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh label list --repo owner/repo --limit 1000 --json name", {
    stdout = label_stdout or "[]",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_codex_findings(stdout, exit_code)
  t.mock_command("codex exec", {
    stdout = stdout,
    stderr = exit_code == 0 and "" or "codex timeout",
    exit_code = exit_code or 0,
  })
end

return {
]] .. body .. [[
}
]]
end

return H
