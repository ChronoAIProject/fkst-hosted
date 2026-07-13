local t = fkst.test

local system_path = "/usr/bin:/bin"
local fixture_prefix = "/tmp/fkst-run-graph-dlq."

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\"'\"'") .. "'"
end

local function command_output(command)
  local handle = assert(io.popen(command .. " 2>&1"))
  local output = handle:read("*a")
  local ok = handle:close()
  return output, ok ~= false and ok ~= nil
end

local function read_command(command)
  local output, ok = command_output(command)
  if not ok then
    error("run_graph DLQ fixture command failed: " .. tostring(command) .. "\n" .. tostring(output))
  end
  return output
end

local function run_command(command)
  read_command(command)
end

local function framework_bin()
  local bin = os.getenv("BIN") or ""
  if bin == "" then
    error("run_graph DLQ fixture requires BIN")
  end
  return bin
end

local function write_file(path, body)
  file.write(path, body)
end

local function write_fixture(root)
  local package_root = root .. "/packages/dlq-fixture"
  run_command("mkdir -p " .. shell_quote(package_root .. "/departments/worker"))
  run_command("mkdir -p " .. shell_quote(package_root .. "/tests"))

  write_file(root .. "/fkst.workspace.toml", [[
[workspace]
units = ["packages/*"]
packages = ["packages/*"]
libraries = []
]])
  write_file(package_root .. "/fkst.toml", [[
kind = "package"
name = "dlq-fixture"

[code]
root = "."
]])
  write_file(package_root .. "/departments/worker/main.lua", [[
local M = {}

M.spec = {
  consumes = { "jobs" },
  produces = {},
  stall_window = "1s",
}

function M.pipeline(_event)
  error("fixture delivery failure")
end

return M
]])
  write_file(package_root .. "/tests/run_graph_failure_test.lua", [[
local t = fkst.test

local function failed_delivery(trace)
  t.eq(trace.status, "quiescent")
  t.eq(#trace.steps, 1)

  local step = trace.steps[1]
  t.eq(step.queue, "dlq-fixture.jobs")
  t.eq(step.consumer, "dlq-fixture.worker")
  t.eq(step.status, "error")
  t.eq(step.exit_code, 1)
  t.is_true(type(step.delivery_id) == "string" and step.delivery_id ~= "")
  t.is_true(tostring(step.error):find("fixture delivery failure", 1, true) ~= nil)
  return step.delivery_id
end

return {
  test_run_graph_reports_attempt_or_terminal_dlq_from_real_store = function()
    local trace = t.run_graph({
      queue = "jobs",
      payload = {
        schema = "dlq-fixture.v1",
        dedup_key = "fixture/job/1",
      },
      source_ref = {
        kind = "external",
        reference = "fixture#job/1",
      },
    }, { max_steps = 2 })

    local delivery_id = failed_delivery(trace)
    if os.getenv("FKST_EXPECT_TERMINAL_DLQ") == "1" then
      t.eq(trace.final.pending, 0, "terminal delivery remained pending: " .. delivery_id)
      t.eq(trace.final.deliveries, 0, "terminal delivery remained live: " .. delivery_id)
      t.eq(trace.final.dead_letters, 1, "terminal delivery did not reach the DLQ: " .. delivery_id)
    else
      t.eq(trace.final.pending, 1, "first failed attempt was not retained: " .. delivery_id)
      t.eq(trace.final.deliveries, 1, "first failed attempt disappeared: " .. delivery_id)
      t.eq(trace.final.dead_letters, 0, "first failed attempt was misclassified as terminal: " .. delivery_id)
    end
  end,
}
]])
  return package_root
end

local function run_fixture(bin, root, package_root, max_attempts, expect_terminal)
  local case_root = root .. "/case-" .. tostring(max_attempts)
  local command = table.concat({
    "PATH=" .. shell_quote(system_path),
    "FKST_RUNTIME_ROOT=" .. shell_quote(case_root .. "/runtime"),
    "FKST_DURABLE_ROOT=" .. shell_quote(case_root .. "/durable"),
    "FKST_RETRY_DEFAULT_MAX_ATTEMPTS=" .. shell_quote(max_attempts),
    "FKST_RETRY_DEFAULT_BASE=" .. shell_quote("1s"),
    "FKST_RETRY_DEFAULT_CAP=" .. shell_quote("1s"),
    "FKST_EXPECT_TERMINAL_DLQ=" .. shell_quote(expect_terminal and "1" or "0"),
    shell_quote(bin),
    "test",
    "--project-root", shell_quote(root),
    "--package-root", shell_quote(package_root),
  }, " ")
  return read_command(command)
end

local function remove_fixture(root)
  if root:sub(1, #fixture_prefix) ~= fixture_prefix then
    error("refusing to remove unexpected fixture root: " .. tostring(root))
  end
  run_command("rm -rf " .. shell_quote(root))
end

return {
  test_run_graph_characterizes_terminal_dlq_exhaustion = function()
    local root = read_command("mktemp -d " .. shell_quote(fixture_prefix .. "XXXXXX")):gsub("%s+$", "")
    local ok, err = pcall(function()
      local bin = framework_bin()
      local package_root = write_fixture(root)

      run_fixture(bin, root, package_root, 2, false)
      run_fixture(bin, root, package_root, 1, true)
    end)

    local cleanup_ok, cleanup_err = pcall(remove_fixture, root)
    if not ok then
      error(err)
    end
    if not cleanup_ok then
      error(cleanup_err)
    end
  end,
}
