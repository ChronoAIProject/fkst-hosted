local base_ids = require("devloop.base_ids")
local parsers_pr = require("devloop.parsers.pr")
local core = require("core")
local saga = require("workflow.saga")
local github = require("devloop.github_factory").production_handle
local git_adapter = require("forge.git").production_handle
local config = require("devloop.config")
local devloop_logging = require("devloop.logging")
local strings = require("contract.strings")
local rollup_health = require("core" .. ".rollup_health")

local spec = {
  consumes = { "devloop_rollup_ready" },
  produces = { "github-devloop.devloop_liveness_tick" },
  stall_window = "5m",
}

local function done(_event)
  return false
end

local function log_skip(payload, reason)
  devloop_logging.log_line("info", "rollup_merge", "rollup", "GATE", {
    "repo=" .. tostring(devloop_logging.payload_field(payload, "repo")),
    "pr=" .. tostring(devloop_logging.payload_field(payload, "pr_number")),
    "outcome=skip",
    "reason=" .. tostring(reason),
  })
end

local function log_runtime_gate_skip(payload, reason, detail)
  devloop_logging.log_line("info", "rollup_merge", "rollup", "GATE", {
    "repo=" .. tostring(devloop_logging.payload_field(payload, "repo")),
    "pr=" .. tostring(devloop_logging.payload_field(payload, "pr_number")),
    "outcome=skip",
    "reason=" .. tostring(reason),
    "detail=" .. tostring(detail),
  })
end

local function unsupported_payload_error(payload, reason)
  return "github-devloop: rollup-ready-payload-invalid: rollup_merge unsupported devloop_rollup_ready payload: dedup_key="
    .. tostring(devloop_logging.payload_field(payload, "dedup_key"))
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

local function integration_head_soak_verdict(payload, pr)
  local soak_minutes = config.rollup_runtime_soak_minutes()
  local git = git_adapter("github-devloop-integration.rollup_merge")
  local fetch = git.fetch_branch("origin", payload.integration_branch, 60)
  if fetch.exit_code ~= 0 then
    return false, "runtime-soak-fetch-failed:" .. tostring(fetch.stderr)
  end
  local head = git.fetch_head_commit(30)
  if head.exit_code ~= 0 then
    return false, "runtime-soak-head-read-failed:" .. tostring(head.stderr)
  end
  local head_sha = strings.trim(head.stdout)
  if head_sha == "" then
    return false, "runtime-soak-head-missing"
  end
  if tostring(head_sha) ~= tostring(pr and pr.head_sha or "") then
    return false, "runtime-soak-head-changed:head=" .. tostring(head_sha)
  end
  return rollup_health.observe_soak_verdict(pr and pr.comments or {}, head_sha, soak_minutes * 60, now())
end

local function runtime_stability_gate(payload, pr)
  local observe_ok, observe_reason, observe_detail = rollup_health.runtime_observe_gate(rollup_health.observe_runtime_health())
  if not observe_ok then
    return false, observe_reason, observe_detail
  end
  local ok, soaked, reason = pcall(function()
    local soak_ok, soak_reason = integration_head_soak_verdict(payload, pr)
    return soak_ok, soak_reason
  end)
  if not ok then
    return false, "runtime-soak-unavailable", soaked
  end
  if soaked ~= true then
    return false, "runtime-soak-unready", reason
  end
  return true, "runtime-stable"
end

local function act(event)
  local payload = event.payload or {}
  local supported, unsupported_reason = core.validate_rollup_ready(payload)
  if not supported then
    devloop_logging.log_entry("rollup_merge", event, "rollup", devloop_logging.payload_field(payload, "dedup_key"))
    log_skip(payload, "unsupported-payload")
    error(unsupported_payload_error(payload, unsupported_reason))
  end

  devloop_logging.log_entry("rollup_merge", event, "rollup", payload.dedup_key)
  with_lock(core.rollup_lock_key(payload.repo, payload.upstream_branch, payload.integration_branch), function()
    if config.write_mode() ~= "real" then
      log_skip(payload, "dry-run")
      return
    end

    local viewed = github("github-devloop-integration.rollup_merge").gh_pr_view_merge(payload.repo, payload.pr_number, 30)
    if viewed.exit_code ~= 0 then
      error("github-devloop: gh-pr-view-failed: gh rollup PR view failed: " .. tostring(viewed.stderr))
    end
    local pr = parsers_pr.parse_pr_view_merge(viewed.stdout)
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
      before_merge = function(rechecked_pr)
        local stable, stability_reason, stability_detail = runtime_stability_gate(payload, rechecked_pr)
        if not stable then
          return false, {
            reason = stability_reason or "runtime-stability-gate",
            detail = stability_detail,
          }
        end
        return true, "runtime-stable"
      end,
    })
    if not merged then
      if type(reason) == "table" and reason.reason ~= nil then
        log_runtime_gate_skip(payload, reason.reason, reason.detail)
      else
        log_skip(payload, reason)
      end
      return
    end
    devloop_logging.log_apply("rollup_merge", "rollup", "rollup-merged", payload.head_sha, {}, {})
    devloop_logging.log_raise(
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
  wrap = devloop_logging.wrap_pipeline_failure,
  name = "rollup_merge",
})
