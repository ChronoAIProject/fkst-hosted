local t = fkst.test

local fixture_prefix = "/tmp/fkst-startup-liveness-supervise."
local system_path = "/usr/bin:/bin"
local wait_seconds = 20

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
    error("startup liveness supervise fixture command failed: " .. tostring(command) .. "\n" .. tostring(output))
  end
  return output
end

local function run_command(command)
  read_command(command)
end

local function framework_bin()
  local bin = os.getenv("BIN") or ""
  if bin == "" then
    error("startup liveness supervise fixture requires BIN")
  end
  return bin
end

local function write_file(path, body)
  file.write(path, body)
end

local function process_alive(pid)
  local _, ok = command_output("kill -0 " .. tostring(pid))
  return ok
end

local function wait_until(description, probe)
  local last = nil
  for _ = 1, wait_seconds * 10 do
    local value, detail = probe()
    last = detail or last
    if value ~= nil and value ~= false then
      return value
    end
    os.execute("sleep 0.1")
  end
  error("timed out waiting for " .. description .. (last and ("\n" .. tostring(last)) or ""))
end

local function stop_process(pid)
  if not process_alive(pid) then
    return
  end
  command_output("kill -TERM " .. tostring(pid))
  wait_until("supervise process to exit", function()
    return not process_alive(pid)
  end)
end

local function supervise_logs(root)
  return command_output(
    "cat " .. shell_quote(root .. "/supervise.stdout") .. " " .. shell_quote(root .. "/supervise.stderr")
      .. "; find " .. shell_quote(root .. "/runtime/logs/framework-child")
      .. " -type f -name '*.log' -exec cat {} +"
  )
end

local function write_fixture(root)
  local gate = root .. "/first-attempt"
  local effects = root .. "/effects.log"
  local department = root .. "/departments/liveness_scan"
  run_command("mkdir -p " .. shell_quote(department))
  run_command("mkdir -p " .. shell_quote(root .. "/raisers"))
  run_command("git -C " .. shell_quote(root) .. " init -q -b main")

  write_file(root .. "/fkst.toml", [[
kind = "package"
name = "startup-liveness-fixture"

[code]
root = "."
]])
  write_file(root .. "/fkst.workspace.toml", [[
[workspace]
units = ["."]
]])
  write_file(root .. "/raisers/liveness_startup.lua", [[
return {
  type = "file_watch",
  glob = ".git/HEAD",
  produces = "devloop_liveness_tick",
}
]])

  local effect_command = "if mkdir " .. shell_quote(gate)
    .. " 2>/dev/null; then exit 75; fi; printf 'recovered\\n' >> " .. shell_quote(effects)
  write_file(department .. "/main.lua", string.format([[
local M = {}

M.spec = {
  consumes = { "devloop_liveness_tick" },
  produces = {},
  stall_window = "30s",
  retry = { max_attempts = 6, base = "5s", cap = "60s" },
}

function M.pipeline(event)
  local path = tostring(event.payload and event.payload.path or "")
  if path:sub(-9) ~= ".git/HEAD" then
    error("startup liveness fixture received a non-Git activation: " .. path)
  end
  local result = exec_sync({ cmd = %q, timeout = 5 })
  if result.exit_code ~= 0 then
    error("github-devloop: liveness-scan-issue-list-failed: transient fixture failure")
  end
  log.info("startup-liveness-recovered path=" .. path)
end

return M
]], effect_command))
  return effects, gate
end

local function start_supervise(bin, root, durable_root)
  local command = table.concat({
    "PATH=" .. shell_quote(system_path),
    "FKST_RUNTIME_ROOT=" .. shell_quote(root .. "/runtime"),
    "FKST_DURABLE_ROOT=" .. shell_quote(durable_root),
    "FKST_RATE_POOL_ROOT=" .. shell_quote(durable_root .. "/rate-pools"),
    shell_quote(bin),
    "supervise",
    "--project-root", shell_quote(root),
    "--package-root", shell_quote(root),
    "--framework-bin", shell_quote(bin),
    ">" .. shell_quote(root .. "/supervise.stdout"),
    "2>" .. shell_quote(root .. "/supervise.stderr"),
    "& printf '%s\\n' \"$!\"",
  }, " ")
  local output = read_command(command)
  local pid = tonumber(output:match("(%d+)"))
  if pid == nil then
    error("startup liveness supervise fixture did not return a pid: " .. tostring(output))
  end
  return pid
end

local function remove_fixture(root)
  if root:sub(1, #fixture_prefix) ~= fixture_prefix then
    error("refusing to remove unexpected startup liveness fixture: " .. tostring(root))
  end
  run_command("rm -rf " .. shell_quote(root))
end

return {
  test_supervise_scans_git_head_immediately_and_retries_the_same_liveness_seam = function()
    local root = read_command("mktemp -d " .. shell_quote(fixture_prefix .. "XXXXXX")):gsub("%s+$", "")
    local active_pid = nil
    local ok, err = pcall(function()
      local effects, gate = write_fixture(root)
      local bin = framework_bin()
      local started_at = os.time()
      active_pid = start_supervise(bin, root, root .. "/durable")

      local recovered = wait_until("startup liveness retry to recover", function()
        local output, read = command_output("cat " .. shell_quote(effects))
        if read and output == "recovered\n" then
          return output
        end
        local logs = supervise_logs(root)
        return nil, logs
      end)
      local elapsed = os.time() - started_at

      t.eq(recovered, "recovered\n")
      local _, first_attempt_seen = command_output("test -d " .. shell_quote(gate))
      t.eq(first_attempt_seen, true)
      t.is_true(elapsed >= 4, "retry completed before the declared 5s backoff")
      t.is_true(elapsed < 15, "startup recovery exceeded the bounded local retry window")
      os.execute("sleep 1")
      t.eq(tonumber(read_command("wc -l < " .. shell_quote(effects)):match("(%d+)")), 1)

      stop_process(active_pid)
      active_pid = nil
    end)

    if active_pid ~= nil then
      pcall(stop_process, active_pid)
    end
    local cleanup_ok, cleanup_err = pcall(remove_fixture, root)
    if not ok then
      error(err)
    end
    if not cleanup_ok then
      error(cleanup_err)
    end
  end,
}
