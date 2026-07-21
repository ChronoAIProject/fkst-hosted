local devloop_base = require("devloop.base")
local t = fkst.test
local core = require("core")
local gh_argv = require("testkit.gh_argv_mock")
gh_argv.install(t, core)

local repo = "owner/repo"

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\"'\"'") .. "'"
end

local function read_command(command)
  local handle = assert(io.popen(command .. " 2>&1"))
  local output = handle:read("*a")
  local ok = handle:close()
  if ok == false or ok == nil then
    error("startup liveness fire_raiser fixture command failed: " .. tostring(command) .. "\n" .. tostring(output))
  end
  return output
end

local function composed_project_root()
  local runtime_root = os.getenv("FKST_RUNTIME_ROOT") or ""
  if runtime_root == "" then
    error("startup liveness fire_raiser fixture requires FKST_RUNTIME_ROOT")
  end
  local manifests = read_command(
    "find " .. shell_quote(runtime_root) .. " -maxdepth 2 -name fkst.workspace.toml -print"
  )
  local roots = {}
  for manifest in manifests:gmatch("[^\n]+") do
    table.insert(roots, (manifest:gsub("/fkst%.workspace%.toml$", "")))
  end
  if #roots ~= 1 then
    error("expected one composed test project root, found " .. tostring(#roots))
  end
  return roots[1]
end

local function mock_empty_board()
  t.mock_command(devloop_base.read_env_command("FKST_GITHUB_REPO"), {
    stdout = repo,
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), {
    stdout = "fkst-test-bot",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(core.gh_issue_list_observe_cmd(repo), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

return {
  test_startup_raiser_uses_real_git_project_root_and_source_semantics = function()
    local project_root = composed_project_root()
    read_command("git -C " .. shell_quote(project_root) .. " init -q -b main")
    mock_empty_board()

    local trace = t.fire_raiser("liveness_startup", {
      fixture = project_root .. "/.git/HEAD",
    })

    t.eq(trace.source_ref.kind, "file_watch")
    t.is_true(trace.source_ref.reference:find("/.git/HEAD/len/", 1, true) ~= nil)
    t.is_true(trace.source_payload.path:sub(-9) == ".git/HEAD")
    t.eq(trace.routed_to[1], "github-devloop.liveness_scan")
    t.eq(trace.consumer_result.status, "accepted")
    t.eq(#trace.raised, 0)
  end,
}
