-- Fire-raiser fixture harness for workflow-security. Mirrors the archaudit fixture:
-- it stages an isolated workspace (this package + its libs + a minimal github-proxy
-- stub whose departments consume the request queues this package produces), writes a
-- child test that fires the cron raiser through the real framework, and asserts the
-- producer-liveness trace. Runs only in CI, where BIN points at the framework binary.
local H = {}

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\"'\"'") .. "'"
end

local function read_command(command)
  local handle = assert(io.popen(command .. " 2>&1"))
  local output = handle:read("*a")
  local ok = handle:close()
  if ok == false or ok == nil then
    error("workflow-security: fixture-command-failed: " .. tostring(command) .. "\n" .. tostring(output))
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
  return (read_command("mktemp -d " .. shell_quote("/tmp/fkst-workflow-security-fire-" .. tostring(name) .. ".XXXXXX")):gsub("%s+$", ""))
end

local function copy_dir(src, dst)
  run_command("mkdir -p " .. shell_quote(dst))
  run_command("cp -R " .. shell_quote(src) .. "/. " .. shell_quote(dst) .. "/")
end

local function write_stub_department(root, name, queue)
  local dir = root .. "/packages/github-proxy/departments/" .. name
  run_command("mkdir -p " .. shell_quote(dir))
  file.write(dir .. "/main.lua", table.concat({
    "local M = {}",
    "M.spec = {",
    "  consumes = { " .. string.format("%q", queue) .. " },",
    "  published_seam = { " .. string.format("%q", queue) .. " },",
    "  stall_window = \"30s\",",
    "}",
    "function M.pipeline(_event)",
    "end",
    "return M",
  }, "\n") .. "\n")
end

function H.setup_workspace(name, child_test)
  local root = temp_root(name)
  local source = repo_root()
  file.write(root .. "/fkst.workspace.toml", '[workspace]\nunits = ["packages/*", "libraries/*"]\n')
  for _, lib in ipairs({ "contract", "workflow", "testkit", "forge", "devloop" }) do
    copy_dir(source .. "/libraries/" .. lib, root .. "/libraries/" .. lib)
  end
  copy_dir(source .. "/packages/workflow-security", root .. "/packages/workflow-security")
  run_command("rm -rf " .. shell_quote(root .. "/packages/workflow-security/tests"))
  file.write(root .. "/packages/github-proxy/fkst.toml", "kind = \"package\"\nname = \"github-proxy\"\n\n[code]\nroot = \".\"\n")
  write_stub_department(root, "github_issue_create", "github_issue_create_request")
  write_stub_department(root, "github_issue_comment", "github_issue_comment_request")
  run_command("mkdir -p " .. shell_quote(root .. "/packages/workflow-security/tests"))
  file.write(root .. "/packages/workflow-security/tests/fire_raiser_child_test.lua", child_test)
  return root
end

local function framework_bin()
  local bin = os.getenv("BIN") or "/Users/auric/fkst-substrate/target/debug/fkst-framework"
  if bin == "" then
    error("workflow-security: fixture-missing-bin: BIN is required")
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
    shell_quote(root .. "/packages/workflow-security"),
    "--package-root",
    shell_quote(root .. "/packages/workflow-security"),
    "--package-root",
    shell_quote(root .. "/packages/github-proxy"),
  }, " ")
  return read_command(command)
end

function H.fire_raiser_child(body)
  return [[
local t = fkst.test

local function mock_env()
  t.mock_command('printf %s "$FKST_GITHUB_REPO"', { stdout = "owner/repo", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', { stdout = "fkst-test-bot", stderr = "", exit_code = 0 })
  t.mock_command('printf %s "$FKST_WORKFLOW_CATALOG_ROOT"', { stdout = "", stderr = "", exit_code = 0 })
end

return {
]] .. body .. [[
}
]]
end

return H
