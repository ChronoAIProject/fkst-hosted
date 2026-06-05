local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_stuck" },
  produces = {
    "github-proxy.github_issue_label_request",
    "github-proxy.github_issue_comment_request",
  },
  stall_window = "2m",
}

function pipeline(event)
  local stuck = event.payload or {}
  if not core.is_supported_stuck(stuck) then
    return
  end

  local repo, issue_number = core.parse_proposal_id(stuck.proposal_id)
  if repo == nil then
    return
  end

  local lock_key = core.meta_lock_key(stuck.proposal_id)
  if lock_key == nil then
    return
  end

  with_lock(lock_key, function()
    local view = exec_sync({ cmd = core.gh_issue_view_meta_cmd(repo, issue_number), timeout = 30 })
    if view.exit_code ~= 0 then
      error("github-devloop: gh issue meta view failed: " .. tostring(view.stderr))
    end

    local current = core.parse_issue_view_meta(view.stdout)
    if core.has_decision_terminal_label(current.labels) then
      return
    end
    if not core.has_stuck_label(current.labels) then
      error("github-devloop: stuck label not yet visible for meta escalation; retrying")
    end
    if core.has_meta_marker(current.comments, stuck.proposal_id, stuck.dedup_key) then
      return
    end

    local result = spawn_codex_sync({
      prompt = core.build_meta_prompt(stuck.proposal_id, current),
      stall_window = M.spec.stall_window,
    })
    if type(result) ~= "table" or result.exit_code ~= 0 or result.stdout == nil then
      local stderr = type(result) == "table" and result.stderr or "nil result"
      error("github-devloop: meta codex failed: " .. tostring(stderr))
    end

    local parsed = core.parse_meta_action(result.stdout)
    if parsed == nil then
      return
    end

    raise("github-proxy.github_issue_label_request", core.build_meta_label_request(repo, issue_number, stuck, parsed.action))
    raise("github-proxy.github_issue_comment_request", core.build_meta_comment_request(repo, issue_number, stuck, parsed.action, parsed.reason))
  end)
end

return M
