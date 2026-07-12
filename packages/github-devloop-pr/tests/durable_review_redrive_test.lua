local devloop_base = require("devloop.base")

local t = fkst.test
local wait_seconds = 15
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
    error("review redrive durable fixture command failed: " .. tostring(command) .. "\n" .. tostring(output))
  end
  return output
end

local function run_command(command)
  read_command(command)
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
  local detail = last and tostring(last):gsub("%s+", " "):sub(-2000) or ""
  error("timed out waiting for " .. description .. (detail ~= "" and (" detail=" .. detail) or ""))
end

local function process_alive(pid)
  local _, ok = command_output("kill -0 " .. tostring(pid))
  return ok
end

local function stop_process(pid)
  if not process_alive(pid) then
    return
  end
  command_output("kill -TERM " .. tostring(pid))
  wait_until("fixture supervise process to exit", function()
    return not process_alive(pid)
  end)
end

local function framework_bin()
  local bin = os.getenv("BIN") or ""
  if bin == "" then
    error("review redrive durable fixture requires BIN")
  end
  return bin
end

local function write_file(path, body)
  file.write(path, body)
end

local function start_supervise(bin, root, package_root, runtime_root, durable_root)
  local command = table.concat({
    "PATH=" .. shell_quote(system_path),
    "FKST_RUNTIME_ROOT=" .. shell_quote(runtime_root),
    "FKST_DURABLE_ROOT=" .. shell_quote(durable_root),
    "FKST_RATE_POOL_ROOT=" .. shell_quote(durable_root .. "/rate-pools"),
    shell_quote(bin),
    "supervise",
    "--project-root", shell_quote(root),
    "--package-root", shell_quote(package_root),
    "--framework-bin", shell_quote(bin),
    ">" .. shell_quote(root .. "/supervise.stdout"),
    "2>" .. shell_quote(root .. "/supervise.stderr"),
    "& printf '%s\\n' \"$!\"",
  }, " ")
  local output = read_command(command)
  local pid = tonumber(output:match("(%d+)"))
  if pid == nil then
    error("review redrive durable fixture did not return a supervise pid: " .. tostring(output))
  end
  return pid
end

local function write_fixture(root, canonical_dedup, redrive_dedup)
  local package_root = root .. "/packages/consensus"
  run_command("mkdir -p " .. shell_quote(package_root .. "/departments/initial"))
  run_command("mkdir -p " .. shell_quote(package_root .. "/departments/redrive"))
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
  local function seed_department(queue, dedup_key)
    return string.format([[
local M = {}
M.spec = {
  consumes = { %q },
  produces = { "proposal" },
  stall_window = "1s",
}
function M.pipeline(_event)
  raise("proposal", {
    schema = "consensus.proposal.v1",
    proposal_id = "github-devloop/pr-review/owner-repo/7/version/def456",
    dedup_key = %q,
    source_ref = { kind = "external", ref = "owner/repo#pr/7" },
  })
end
return M
]], queue, dedup_key)
  end
  write_file(package_root .. "/departments/initial/main.lua", seed_department("initial", canonical_dedup))
  write_file(package_root .. "/departments/redrive/main.lua", seed_department("redrive", redrive_dedup))
  write_file(package_root .. "/departments/decide/main.lua", string.format([[
local M = {}
M.spec = {
  consumes = { "proposal" },
  produces = {},
  stall_window = "1s",
  retry = { max_attempts = 1, base = "1s", cap = "1s" },
}
function M.pipeline(event)
  if event.payload.dedup_key == %q then
    error("fixture canonical delivery fails permanently")
  end
  if event.payload.dedup_key ~= %q then
    error("fixture received an unknown review delivery identity")
  end
  log.info("review-redrive-consumed dedup_key=" .. tostring(event.payload.dedup_key))
end
return M
]], canonical_dedup, redrive_dedup))
  write_file(package_root .. "/raisers/initial.lua", string.format([[
return { type = "file_watch", glob = %q, produces = "initial" }
]], root .. "/initial.trigger"))
  write_file(package_root .. "/raisers/redrive.lua", string.format([[
return { type = "file_watch", glob = %q, produces = "redrive" }
]], root .. "/redrive.trigger"))
  return package_root
end

local function observe(bin, durable_root)
  return command_output(shell_quote(bin) .. " observe --durable-root " .. shell_quote(durable_root) .. " --json")
end

local function logs(root)
  return command_output("cat " .. shell_quote(root .. "/supervise.stdout") .. " " .. shell_quote(root .. "/supervise.stderr"))
end

local function remove_fixture(root)
  local prefix = "/tmp/fkst-review-redrive."
  if root:sub(1, #prefix) ~= prefix then
    error("refusing to remove unexpected fixture root: " .. tostring(root))
  end
  run_command("rm -rf " .. shell_quote(root))
end

return {
  test_terminal_review_delivery_redrives_to_consensus_decide_with_fresh_identity = function()
    local review_id = devloop_base.pr_review_proposal_id("owner/repo", 7, "version", "def456")
    local canonical_dedup = devloop_base.pr_review_proposal_dedup_key(review_id)
    local redrive_dedup = devloop_base.pr_review_redrive_delivery_dedup_key(
      review_id,
      "restart-liveness:v2/reviewing/reviewing.active/state-entry/1",
      1
    )
    t.is_true(canonical_dedup ~= redrive_dedup)

    local root = read_command("mktemp -d " .. shell_quote("/tmp/fkst-review-redrive.XXXXXX")):gsub("%s+$", "")
    local active_pid = nil
    local ok, err = pcall(function()
      local bin = framework_bin()
      local durable_root = root .. "/durable"
      local package_root = write_fixture(root, canonical_dedup, redrive_dedup)
      write_file(root .. "/initial.trigger", "initial\n")
      active_pid = start_supervise(bin, root, package_root, root .. "/runtime", durable_root)

      wait_until("canonical review delivery to become terminal", function()
        local output, observed = observe(bin, durable_root)
        if observed
          and output:find('"queue": "consensus.proposal"', 1, true) ~= nil
          and output:find('/dedup/', 1, true) ~= nil
          and output:find('"permanent": true', 1, true) ~= nil then
          return output
        end
        local supervise_logs = logs(root)
        return nil, output .. "\nsupervise logs:\n" .. supervise_logs
      end)

      stop_process(active_pid)
      active_pid = nil
      write_file(root .. "/redrive.trigger", "redrive\n")
      active_pid = start_supervise(bin, root, package_root, root .. "/runtime-redrive", durable_root)
      wait_until("fresh review delivery to reach consensus.decide", function()
        local output = logs(root)
        if output:find("dept=consensus.decide", 1, true) ~= nil
          and output:find("_2F_r_2F_", 1, true) ~= nil
          and output:find("MSG=delivery acked", 1, true) ~= nil then
          return output
        end
        return nil, output
      end)
      local snapshot = observe(bin, durable_root)
      t.is_true(snapshot:find('"queue": "consensus.proposal"', 1, true) ~= nil)
      t.is_true(snapshot:find('"permanent": true', 1, true) ~= nil)
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
