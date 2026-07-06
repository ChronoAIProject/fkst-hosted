local t = fkst.test

local known_sha = "dddddddddddddddddddddddddddddddddddddddd"
local next_sha = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\"'\"'") .. "'"
end

local function read_command(command)
  local handle = assert(io.popen(command .. " 2>&1"))
  local output = handle:read("*a")
  local ok = handle:close()
  if ok == false or ok == nil then
    error("git-branch-detector graph fixture command failed: " .. tostring(command) .. "\n" .. tostring(output))
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
  return (read_command("mktemp -d " .. shell_quote("/tmp/fkst-git-ref-dedup-" .. tostring(name) .. ".XXXXXX")):gsub("%s+$", ""))
end

local function copy_dir(src, dst)
  run_command("mkdir -p " .. shell_quote(dst))
  run_command("cp -R " .. shell_quote(src) .. "/. " .. shell_quote(dst .. "/"))
end

local function write_file(path, body)
  file.write(path, body)
end

local function setup_workspace(name, child_test)
  local root = temp_root(name)
  local source = repo_root()
  write_file(root .. "/fkst.workspace.toml", '[workspace]\nunits = ["packages/*", "libraries/*"]\n')
  for _, lib in ipairs({ "contract", "workflow", "testkit", "forge" }) do
    copy_dir(source .. "/libraries/" .. lib, root .. "/libraries/" .. lib)
  end
  copy_dir(source .. "/packages/git-branch-detector", root .. "/packages/git-branch-detector")
  run_command("rm -rf " .. shell_quote(root .. "/packages/git-branch-detector/tests"))

  run_command("mkdir -p " .. shell_quote(root .. "/packages/git-ref-sink/departments/record"))
  write_file(root .. "/packages/git-ref-sink/fkst.toml", [[
kind = "package"
name = "git-ref-sink"

[code]
root = "."
]])
  write_file(root .. "/packages/git-ref-sink/departments/record/main.lua", [[
local M = {}
M.spec = {
  consumes = { "git-branch-detector.git_ref_changed" },
  produces = {},
  stall_window = "30s",
}
function M.pipeline(_event)
end
return M
]])

  run_command("mkdir -p " .. shell_quote(root .. "/packages/git-branch-detector/tests"))
  write_file(root .. "/packages/git-branch-detector/tests/ref_dedup_child_test.lua", child_test)
  return root
end

local function framework_bin()
  local bin = os.getenv("BIN") or "/Users/auric/fkst-substrate/target/debug/fkst-framework"
  if bin == "" then
    error("git-branch-detector graph fixture requires BIN")
  end
  return bin
end

local function run_child(root)
  local bin = framework_bin()
  local command = table.concat({
    "BIN=" .. shell_quote(bin),
    "FKST_RUNTIME_ROOT=" .. shell_quote(root .. "/runtime"),
    "FKST_DURABLE_ROOT=" .. shell_quote(root .. "/durable"),
    shell_quote(bin),
    "test",
    "--project-root",
    shell_quote(root .. "/packages/git-branch-detector"),
    "--package-root",
    shell_quote(root .. "/packages/git-branch-detector"),
    "--package-root",
    shell_quote(root .. "/packages/git-ref-sink"),
  }, " ")
  return read_command(command)
end

local function child_test(body)
  return [=[
local t = fkst.test

local function event(reference)
  return {
    queue = "git_ref_poll_tick",
    payload = {
      schema = "git-branch-detector.ref-poll-tick.v1",
    },
    source_ref = {
      kind = "cron",
      reference = reference,
    },
  }
end

local function mock_watch_refs(value)
  t.mock_command('printf %s "$FKST_GIT_WATCH_REFS"', {
    stdout = value,
    stderr = "",
    exit_code = 0,
  })
end

local function mock_git(sha)
  t.mock_command("git ls-remote 'origin' 'refs/heads/main'", {
    stdout = sha .. "\trefs/heads/main\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("git ls-remote origin refs/heads/main", {
    stdout = sha .. "\trefs/heads/main\n",
    stderr = "",
    exit_code = 0,
  })
end

local function sink_deliveries(trace)
  local result = {}
  for _, step in ipairs(trace.steps or {}) do
    if step.queue == "git-branch-detector.git_ref_changed" and step.consumer == "git-ref-sink.record" then
      table.insert(result, step)
    end
  end
  return result
end

return {
]=] .. body .. [=[
}
]=]
end

return {
  test_same_sha_dedupes_through_durable_delivery_identity = function()
    local root = setup_workspace("same", child_test([[
  test_same_sha = function()
    mock_watch_refs("origin#main origin#main")
    mock_git("]] .. known_sha .. [[")
    mock_git("]] .. known_sha .. [[")
    local trace = t.run_graph(event("tick/same"), { max_steps = 4 })

    local delivered = sink_deliveries(trace)
    t.eq(#trace.steps[1].raises, 2)
    t.eq(trace.steps[1].raises[1].payload.dedup_key, "git-ref/origin#main#]] .. known_sha .. [[")
    t.eq(trace.steps[1].raises[2].payload.dedup_key, trace.steps[1].raises[1].payload.dedup_key)
    t.eq(#delivered, 1)
    t.is_true(delivered[1].delivery_id:find("/dedup/git-ref_2F_origin_23_main_23_]] .. known_sha .. [[", 1, true) ~= nil)
  end,
]]))
    local output = run_child(root)
    t.is_true(output:find("1 passed, 0 failed", 1, true) ~= nil, output)
  end,

  test_new_sha_creates_fresh_durable_delivery_identity = function()
    local root = setup_workspace("new", child_test([[
  test_new_sha = function()
    mock_watch_refs("origin#main origin#main")
    mock_git("]] .. known_sha .. [[")
    mock_git("]] .. next_sha .. [[")
    local trace = t.run_graph(event("tick/new"), { max_steps = 4 })

    local delivered = sink_deliveries(trace)
    t.eq(#trace.steps[1].raises, 2)
    t.eq(trace.steps[1].raises[1].payload.dedup_key, "git-ref/origin#main#]] .. known_sha .. [[")
    t.eq(trace.steps[1].raises[2].payload.dedup_key, "git-ref/origin#main#]] .. next_sha .. [[")
    t.eq(#delivered, 2)
    t.is_true(delivered[1].delivery_id ~= delivered[2].delivery_id)
  end,
]]))
    local output = run_child(root)
    t.is_true(output:find("1 passed, 0 failed", 1, true) ~= nil, output)
  end,
}
