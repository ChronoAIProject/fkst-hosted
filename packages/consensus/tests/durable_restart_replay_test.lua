local t = fkst.test

local supervisor_wait_seconds = 15
local system_path = "/usr/bin:/bin"

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
    error("consensus durable restart fixture command failed: " .. tostring(command) .. "\n" .. tostring(output))
  end
  return output
end

local function run_command(command)
  read_command(command)
end

local function repo_root()
  return (read_command("pwd"):gsub("%s+$", ""))
end

local function temp_root()
  return (read_command("mktemp -d " .. shell_quote("/tmp/fkst-consensus-durable-restart.XXXXXX")):gsub("%s+$", ""))
end

local function framework_bin()
  local bin = os.getenv("BIN") or "/Users/auric/fkst-substrate/target/debug/fkst-framework"
  if bin == "" then
    error("consensus durable restart fixture requires BIN")
  end
  return bin
end

local function write_file(path, body)
  file.write(path, body)
end

local function wait_until(description, probe)
  local attempts = supervisor_wait_seconds * 10
  local last = nil
  for _ = 1, attempts do
    local value, detail = probe()
    last = detail or last
    if value ~= nil and value ~= false then
      return value
    end
    os.execute("sleep 0.1")
  end
  error("timed out waiting for " .. description .. (last and ("\n" .. tostring(last)) or ""))
end

local function process_alive(pid)
  local _, ok = command_output("kill -0 " .. tostring(pid))
  return ok
end

local function stop_process(pid, signal)
  if not process_alive(pid) then
    return
  end
  command_output("kill -" .. tostring(signal) .. " " .. tostring(pid))
  wait_until("supervise process " .. tostring(pid) .. " to exit", function()
    if not process_alive(pid) then
      return true
    end
    return nil
  end)
end

local function start_supervise(bin, project_root, package_roots, runtime_root, durable_root, log_prefix)
  local parts = {
    "PATH=" .. shell_quote(system_path),
    "FKST_RUNTIME_ROOT=" .. shell_quote(runtime_root),
    "FKST_DURABLE_ROOT=" .. shell_quote(durable_root),
    "FKST_RATE_POOL_ROOT=" .. shell_quote(durable_root .. "/rate-pools"),
    "FKST_RETRY_DEFAULT_BASE=" .. shell_quote("1s"),
    "FKST_RETRY_DEFAULT_CAP=" .. shell_quote("1s"),
    "FKST_CODEX_PERMIT_SLOTS=1",
    shell_quote(bin),
    "supervise",
    "--project-root",
    shell_quote(project_root),
  }
  for _, package_root in ipairs(package_roots) do
    table.insert(parts, "--package-root")
    table.insert(parts, shell_quote(package_root))
  end
  table.insert(parts, "--framework-bin")
  table.insert(parts, shell_quote(bin))
  table.insert(parts, ">" .. shell_quote(log_prefix .. ".stdout"))
  table.insert(parts, "2>" .. shell_quote(log_prefix .. ".stderr"))
  table.insert(parts, "& printf '%s\\n' \"$!\"")

  local output = read_command(table.concat(parts, " "))
  local pid = tonumber(output:match("(%d+)"))
  if pid == nil then
    error("consensus durable restart fixture did not return a supervise pid: " .. tostring(output))
  end
  return pid
end

local function write_producer_fixture(root, stale_worktree)
  local package_root = root .. "/packages/consensus"
  run_command("mkdir -p " .. shell_quote(package_root .. "/departments/seed"))
  run_command("mkdir -p " .. shell_quote(package_root .. "/departments/decide"))
  run_command("mkdir -p " .. shell_quote(package_root .. "/raisers"))
  write_file(root .. "/fkst.workspace.toml", [[
[workspace]
units = ["packages/*"]
packages = ["packages/*"]
libraries = []
]])
  write_file(package_root .. "/fkst.toml", [[
kind = "package"
name = "consensus"

[code]
root = "."
]])
  write_file(package_root .. "/departments/seed/main.lua", string.format([[
local M = {}

M.spec = {
  consumes = { "seed" },
  produces = { "proposal" },
  stall_window = "5s",
}

function M.pipeline(_event)
  raise("proposal", {
    schema = "consensus.proposal.v1",
    proposal_id = "durable-restart-proposal",
    title = "Judge a proposal replayed after restart",
    body = "Verify that the durable proposal remains consumable.",
    content_fetch = "fetch-source --ref demo/consensus/restart --full",
    angles = { "teleology", "parsimony", "fidelity" },
    dedup_key = "durable-restart-proposal/v1",
    source_ref = { kind = "proposal", ref = "demo/consensus/restart" },
    worktree = %q,
  })
end

return M
]], stale_worktree))
  write_file(package_root .. "/departments/decide/main.lua", [[
local M = {}

M.spec = {
  consumes = { "proposal" },
  produces = {},
  stall_window = "1s",
}

function M.pipeline(_event)
  local result = exec_sync({
    cmd = 'while kill -0 "$FKST_SUPERVISOR_PID" 2>/dev/null; do sleep 0.1; done',
    timeout = 30,
  })
  if result.exit_code ~= 0 then
    error("fixture subscriber failed while waiting for the first supervisor to stop")
  end
end

return M
]])
  local input = root .. "/proposal.trigger"
  write_file(package_root .. "/raisers/proposal.lua", string.format([[
return {
  type = "file_watch",
  glob = %q,
  produces = "seed",
}
]], input))
  write_file(input, "ready\n")
  return package_root
end

local function observe_durable(bin, durable_root)
  return command_output(
    shell_quote(bin) .. " observe --durable-root " .. shell_quote(durable_root) .. " --json"
  )
end

local function write_replay_host_fixture(root)
  local host_root = root .. "/replay-host"
  run_command("mkdir -p " .. shell_quote(host_root .. "/.git"))
  run_command("mkdir -p " .. shell_quote(host_root .. "/departments/dormant_source"))
  run_command("mkdir -p " .. shell_quote(host_root .. "/raisers"))
  write_file(host_root .. "/fkst.workspace.toml", [[
[workspace]
units = ["."]
]])
  write_file(host_root .. "/fkst.toml", [[
kind = "package.composed"
name = "restart-host"

[code]
root = "."

[event_deps]
packages = ["consensus"]
]])
  write_file(host_root .. "/departments/dormant_source/main.lua", [[
local M = {}

M.spec = {
  consumes = { "restart_seed" },
  produces = { "consensus.proposal" },
  stall_window = "5s",
}

function M.pipeline(_event)
end

return M
]])
  write_file(host_root .. "/raisers/dormant_source.lua", string.format([[
return {
  type = "file_watch",
  glob = %q,
  produces = "restart_seed",
}
]], host_root .. "/never/*.trigger"))
  return host_root
end

local function adoption_records(runtime_root)
  return command_output(
    "find " .. shell_quote(runtime_root .. "/logs/codex-adoption")
      .. " -type f -name status.json -exec cat {} +"
  )
end

local function remove_fixture(root)
  local prefix = "/tmp/fkst-consensus-durable-restart."
  if root:sub(1, #prefix) ~= prefix then
    error("refusing to remove unexpected fixture root: " .. tostring(root))
  end
  run_command("rm -rf " .. shell_quote(root))
end

return {
  test_durable_proposal_replays_after_supervise_restart_with_fresh_runtime_root = function()
    local _, codex_on_system_path = command_output("PATH=" .. shell_quote(system_path) .. " command -v codex")
    t.eq(codex_on_system_path, false)

    local root = temp_root()
    local active = {}
    local ok, err = pcall(function()
      local bin = framework_bin()
      local source = repo_root()
      local durable_root = root .. "/durable"
      local old_runtime = root .. "/runtime-before-restart"
      local retired_runtime = root .. "/runtime-before-restart-retired"
      local new_runtime = root .. "/runtime-after-restart"
      local stale_worktree = old_runtime .. "/worktrees/review-42"
      run_command("mkdir -p " .. shell_quote(stale_worktree .. "/.git"))
      local producer = write_producer_fixture(root, stale_worktree)
      local replay_host = write_replay_host_fixture(root)

      local first_pid = start_supervise(
        bin,
        root,
        { producer },
        old_runtime,
        durable_root,
        root .. "/supervise-before-restart"
      )
      active[first_pid] = true
      wait_until("in-flight durable consensus proposal", function()
        local output, observed = observe_durable(bin, durable_root)
        if observed
          and output:find('"queue": "consensus.proposal"', 1, true) ~= nil
          and output:find('"dedup_key": "durable-restart-proposal/v1"', 1, true) ~= nil
          and output:find('"status": "in-flight"', 1, true) ~= nil then
          return output
        end
        local supervise_stderr = command_output("cat " .. shell_quote(root .. "/supervise-before-restart.stderr"))
        return nil, output .. "\nsupervise stderr:\n" .. supervise_stderr
      end)

      stop_process(first_pid, "KILL")
      active[first_pid] = nil
      run_command("mv " .. shell_quote(old_runtime) .. " " .. shell_quote(retired_runtime))
      local _, stale_exists = command_output("test -e " .. shell_quote(stale_worktree))
      t.eq(stale_exists, false)

      local second_pid = start_supervise(
        bin,
        replay_host,
        { source .. "/packages/consensus" },
        new_runtime,
        durable_root,
        root .. "/supervise-after-restart"
      )
      active[second_pid] = true
      local records = wait_until("replayed consensus dispatch from the replacement runtime root", function()
        local output, read = adoption_records(new_runtime)
        if read and output:find(" -C . ", 1, true) ~= nil then
          return output
        end
        local supervise_stderr = command_output("cat " .. shell_quote(root .. "/supervise-after-restart.stderr"))
        return nil, output .. "\nsupervise stderr:\n" .. supervise_stderr
      end)

      t.is_nil(records:find(stale_worktree, 1, true))
      t.is_true(records:find(" -C . ", 1, true) ~= nil)
      stop_process(second_pid, "TERM")
      active[second_pid] = nil
    end)

    for pid in pairs(active) do
      pcall(stop_process, pid, "KILL")
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
