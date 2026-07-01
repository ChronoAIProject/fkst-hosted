local base_ids = require("devloop.base_ids")
local parsers_pr = require("devloop.parsers.pr")
local core = require("core")
local saga = require("workflow.saga")
local github = require("forge.github").production_handle
local config = require("devloop.config")

local spec = {
  consumes = { "devloop_rollup_ready" },
  produces = { "github-devloop.devloop_liveness_tick" },
  stall_window = "5m",
}

local function done(_event)
  return false
end

local function log_skip(payload, reason)
  core.log_line("info", "rollup_merge", "rollup", "GATE", {
    "repo=" .. tostring(core.payload_field(payload, "repo")),
    "pr=" .. tostring(core.payload_field(payload, "pr_number")),
    "outcome=skip",
    "reason=" .. tostring(reason),
  })
end

local function unsupported_payload_error(payload, reason)
  return "github-devloop: rollup_merge unsupported devloop_rollup_ready payload: dedup_key="
    .. tostring(core.payload_field(payload, "dedup_key"))
    .. " reason="
    .. tostring(reason)
end

local function rollup_liveness_tick_payload(payload)
  return {
    schema = "github-devloop.tick.v1",
    repo = payload.repo,
    reason = "rollup-merged",
    source_ref = payload.source_ref,
    dedup_key = base_ids.dedup_key({
      "rollup",
      "liveness-tick",
      tostring(payload.repo),
      tostring(payload.upstream_branch),
      tostring(payload.integration_branch),
      tostring(payload.pr_number),
      tostring(payload.head_sha),
    }),
  }
end

local function act(event)
  local payload = event.payload or {}
  local supported, unsupported_reason = core.validate_rollup_ready(payload)
  if not supported then
    core.log_entry("rollup_merge", event, "rollup", core.payload_field(payload, "dedup_key"))
    log_skip(payload, "unsupported-payload")
    error(unsupported_payload_error(payload, unsupported_reason))
  end

  core.log_entry("rollup_merge", event, "rollup", payload.dedup_key)
  with_lock(core.rollup_lock_key(payload.repo, payload.upstream_branch, payload.integration_branch), function()
    if config.write_mode(core) ~= "real" then
      log_skip(payload, "dry-run")
      return
    end

    local viewed = github("github-devloop-integration.rollup_merge").gh_pr_view_merge(payload.repo, payload.pr_number, 30)
    if viewed.exit_code ~= 0 then
      error("github-devloop: gh rollup PR view failed: " .. tostring(viewed.stderr))
    end
    local pr = parsers_pr.parse_pr_view_merge(core, viewed.stdout)
    if tostring(pr.head_ref_name or "") ~= tostring(payload.integration_branch or "") then
      log_skip(payload, "head-branch-mismatch")
      return
    end
    if tostring(pr.base_ref_name or "") ~= tostring(payload.upstream_branch or "") then
      log_skip(payload, "base-branch-mismatch")
      return
    end
    if not require("forge.merge.shared").is_same_repo_pr_head(pr, payload.repo) then
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
    core.log_raise(
      "rollup_merge",
      "rollup",
      "github-devloop.devloop_liveness_tick",
      rollup_liveness_tick_payload(payload)
    )
  end)
end

return saga.department(spec, {
  done = done,
  act = act,
  wrap = core.wrap_pipeline_failure,
  name = "rollup_merge",
})
