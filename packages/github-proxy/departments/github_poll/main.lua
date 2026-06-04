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

  local result = exec_sync({ cmd = core.gh_issue_list_cmd(repo), timeout = 30 })
  if result.exit_code ~= 0 then
    log.warn("github-proxy: gh issue list failed: " .. tostring(result.stderr))
    return
  end

  local issues = core.parse_issue_list(result.stdout)
  for _, issue in ipairs(issues) do
    local key = core.issue_dedup_key(repo, issue.number, issue.updated_at)
    once("github-proxy:seen:issue:" .. key, function()
      -- At-least-once: once marks only after the callback succeeds. If the
      -- process crashes during raise, the next tick raises again. Downstream
      -- consumers must dedup by payload.dedup_key.
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
    end)
  end
end

return M
