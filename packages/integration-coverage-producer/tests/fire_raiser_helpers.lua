local H = {}

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\"'\"'") .. "'"
end

local function read_command(command)
  local handle = assert(io.popen(command .. " 2>&1"))
  local output = handle:read("*a")
  local ok = handle:close()
  if ok == false or ok == nil then
    error("integration coverage fire_raiser fixture command failed: " .. tostring(command) .. "\n" .. tostring(output))
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
  return (read_command("mktemp -d " .. shell_quote("/tmp/fkst-integration-coverage-fire-raiser-" .. tostring(name) .. ".XXXXXX")):gsub("%s+$", ""))
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
  copy_dir(source .. "/packages/integration-coverage-producer", root .. "/packages/integration-coverage-producer")
  run_command("rm -rf " .. shell_quote(root .. "/packages/integration-coverage-producer/tests"))
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
  run_command("mkdir -p " .. shell_quote(root .. "/packages/integration-coverage-producer/tests"))
  write_file(root .. "/packages/integration-coverage-producer/tests/fire_raiser_child_test.lua", child_test)
  return root
end

local function framework_bin()
  local bin = os.getenv("BIN") or "/Users/auric/fkst-substrate/target/debug/fkst-framework"
  if bin == "" then
    error("integration coverage fire_raiser fixture requires BIN")
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
    shell_quote(root .. "/packages/integration-coverage-producer"),
    "--package-root",
    shell_quote(root .. "/packages/integration-coverage-producer"),
    "--package-root",
    shell_quote(root .. "/packages/github-proxy"),
  }, " ")
  return read_command(command)
end

function H.fire_raiser_child(body)
  return [=[
local t = fkst.test

local checker_fixture = [[
[
  {
    "consumer_dept": "outbound_glue",
    "consumer_pkg": "github-autochrono",
    "edge_id": "autochrono.reply -> github-autochrono.outbound_glue",
    "owner_scope": "platform-owned",
    "producer_dept": "reply",
    "producer_pkg": "autochrono",
    "queue": "autochrono.reply",
    "status": "uncovered-allowlisted"
  }
]
]]

local function mock_env()
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', {
    stdout = "owner/repo",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_checker()
  t.mock_command("tools/check_repo_integration_coverage.py", {
    stdout = checker_fixture,
    stderr = "integration coverage check failed",
    exit_code = 1,
  })
end

local function mock_production_issue_reads()
  t.mock_command("gh issue list --repo owner/repo --state open --limit 100 --json 'number,title,state,labels'", {
    stdout = "[]",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue list --repo owner/repo --state all --limit 100 --search 'coverage-edge-id: autochrono.reply -> github-autochrono.outbound_glue' --json 'number,title,state,author,body,labels,url'", {
    stdout = "[]",
    stderr = "",
    exit_code = 0,
  })
end

return {
]=] .. body .. [=[
}
]=]
end

return H
