local core = require("core")

local M = {}

M.spec = {
  consumes = { "devloop_rollup_ready" },
  produces = {},
  stall_window = "5m",
}

local function log_skip(payload, reason)
  core.log_line("info", "rollup_merge", "rollup", "GATE", {
    "repo=" .. tostring(core.payload_field(payload, "repo")),
    "pr=" .. tostring(core.payload_field(payload, "pr_number")),
    "outcome=skip",
    "reason=" .. tostring(reason),
  })
end

function pipeline(event)
  local payload = event.payload or {}
  if not core.is_supported_rollup_ready(payload) then
    core.log_entry("rollup_merge", event, "rollup", core.payload_field(payload, "dedup_key"))
    log_skip(payload, "unsupported-payload")
    return
  end

  core.log_entry("rollup_merge", event, "rollup", payload.dedup_key)
  with_lock(core.rollup_lock_key(payload.repo, payload.upstream_branch, payload.integration_branch), function()
    if core.write_mode() ~= "real" then
      log_skip(payload, "dry-run")
      return
    end

    local viewed = core.gh_exec({ cmd = core.gh_pr_view_merge_cmd(payload.repo, payload.pr_number), timeout = 30 })
    if viewed.exit_code ~= 0 then
      error("github-devloop: gh rollup PR view failed: " .. tostring(viewed.stderr))
    end
    local pr = core.parse_pr_view_merge(viewed.stdout)
    if tostring(pr.head_ref_name or "") ~= tostring(payload.integration_branch or "") then
      log_skip(payload, "head-branch-mismatch")
      return
    end
    if tostring(pr.base_ref_name or "") ~= tostring(payload.upstream_branch or "") then
      log_skip(payload, "base-branch-mismatch")
      return
    end
    if not core.is_same_repo_pr_head(pr, payload.repo) then
      log_skip(payload, "foreign-head-repository")
      return
    end
    local gate_ok, gate_reason = core.evaluate_ci_merge_gate(pr, {
      repo = payload.repo,
      dept = "rollup_merge",
      proposal_id = "rollup",
    })
    if not gate_ok then
      log_skip(payload, gate_reason)
      return
    end

    local merged, reason = core.run_verified_pr_merge({
      repo = payload.repo,
      pr_number = payload.pr_number,
      head_sha = payload.head_sha,
      head_branch = payload.integration_branch,
      base_branch = payload.upstream_branch,
      dept = "rollup_merge",
      proposal_id = "rollup",
      accept_current_head = true,
      match_head_retry_attempts = 3,
    })
    if not merged then
      log_skip(payload, reason)
      return
    end
    core.log_apply("rollup_merge", "rollup", "rollup-merged", payload.head_sha, {}, {})
  end)
end

pipeline = core.wrap_pipeline_failure("rollup_merge", pipeline)

return M
