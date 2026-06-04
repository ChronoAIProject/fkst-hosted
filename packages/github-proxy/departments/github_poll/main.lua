local core = require("core")

local M = {}

M.spec = {
  consumes = { "github_poll_tick" },
  produces = { "github_issue_seen" },
  stall_window = "30s",
}

function pipeline(_event)
  local repo = core.read_env("FKST_GITHUB_REPO")
  if repo == nil then
    log.warn("github-proxy: FKST_GITHUB_REPO missing; skipping poll")
    return
  end
  local runtime_root = core.read_env("FKST_RUNTIME_ROOT")
  if runtime_root == nil then
    log.warn("github-proxy: FKST_RUNTIME_ROOT missing; skipping poll")
    return
  end

  local result = exec_sync({ cmd = core.gh_issue_list_cmd(repo), timeout = 30 })
  if result.exit_code ~= 0 then
    log.warn("github-proxy: gh issue list failed: " .. tostring(result.stderr))
    return
  end

  local issues = core.parse_issue_list(result.stdout)
  for _, issue in ipairs(issues) do
    local key = core.issue_dedup_key(repo, issue.number, issue.updated_at)
    local marker = core.seen_marker_path(runtime_root, key)
    local marker_dir = marker:match("^(.*)/[^/]*$")
    with_lock("github-proxy-ledger", function()
      if not file.exists(marker) then
        -- At-least-once: if the process crashes after raise and before the empty
        -- marker write, the next tick raises again. Downstream consumers must dedup by
        -- payload.dedup_key.
        raise("github_issue_seen", {
          schema = "github-proxy.v1",
          repo = repo,
          issue_number = issue.number,
          title = issue.title,
          url = issue.url,
          updated_at = issue.updated_at,
          state = issue.state,
          dedup_key = key,
          source = "gh",
        })
        local mkdir = exec_sync({ cmd = core.mkdir_p_cmd(marker_dir), timeout = 30 })
        if mkdir.exit_code ~= 0 then
          log.warn("github-proxy: seen marker mkdir failed: " .. tostring(mkdir.stderr))
          return
        end
        file.write(marker, key .. "\n")
      end
    end)
  end
end

return M
