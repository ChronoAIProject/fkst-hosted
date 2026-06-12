local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_dependency_reconcile_tick" },
  produces = { "github-proxy.github_entity_changed" },
  fanout = { "devloop_dependency_reconcile_tick" },
  stall_window = "30s",
}

local function read_repo()
  local repo = core.read_env("FKST_GITHUB_REPO")
  if repo == nil or not core.issue_ref_round_trips(repo, 1) then
    return nil
  end
  return repo
end

local function build_observe_payload(repo, issue, tick)
  local issue_number = tostring(issue.number or "")
  local updated_at = tostring(issue.updated_at or "")
  return {
    schema = "github-proxy.v1",
    type = "issue",
    repo = repo,
    number = tonumber(issue_number),
    state = issue.state,
    updated_at = updated_at,
    dedup_key = core._dedup_key({
      "dependency-reconcile",
      tostring(repo),
      "issue",
      issue_number,
      updated_at,
      tostring(tick or ""),
    }),
    source = "dependency-reconcile",
    source_ref = core.issue_source_ref(repo, issue_number),
  }
end

function pipeline(event)
  core.log_entry("dependency_reconcile", event, "github-devloop/dependency-reconcile", "tick")
  core.assert_trusted_bot_configured()

  local repo = read_repo()
  if repo == nil then
    core.log_cas_decision("dependency_reconcile", "github-devloop/dependency-reconcile", { state = nil, version = nil }, "tick", "observe", "skip-invalid-repo", "FKST_GITHUB_REPO is missing or invalid")
    return
  end

  local list = core.gh_exec({ cmd = core.gh_issue_list_dependency_reconcile_cmd(repo), timeout = 60 })
  if list.exit_code ~= 0 then
    error("github-devloop: gh dependency reconcile issue list failed: " .. tostring(list.stderr))
  end

  for _, issue in ipairs(core.parse_issue_list_observe(list.stdout)) do
    if core.issue_ref_round_trips(repo, issue.number) then
      local proposal_id = core.proposal_id(repo, issue.number)
      local payload = build_observe_payload(repo, issue, event and event.ts)
      core.log_apply("dependency_reconcile", proposal_id, nil, nil, { add = {}, remove = {} }, {
        "github-proxy.github_entity_changed",
      })
      core.log_raise("dependency_reconcile", proposal_id, "github-proxy.github_entity_changed", payload)
    end
  end
end

return M
