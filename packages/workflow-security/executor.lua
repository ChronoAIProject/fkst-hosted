-- workflow-security: the EXECUTOR seam (raise_step / emit_terminal).
--
-- The kernel owns WHEN a step is materialized and the CAS key; this module owns HOW
-- a security-review step becomes a real artifact. A step is a codex ANALYSIS run
-- (not an issue). raise_step is idempotent on step_ctx.child_dedup_key: a repeat
-- call finds the existing materialization marker and returns "exists"; an in-flight
-- generated marker returns "wait"; otherwise it spawns codex, publishes the step
-- output + a "created" marker as a bot comment, and returns "raised".
--
-- The final step ("file-findings") is where the multi-step pipeline pays off: its
-- codex output is a consolidated findings array which the executor turns into
-- dedup-idempotent github-proxy issue-create requests (labelled fkst-security)
-- before recording its created marker — so re-reconcile never double-files.
local materialization = require("workflow.engine.materialization")
local security_logic = require("security_logic")

local M = {}

local function analysis_comment_body(slot, output, marker_text)
  return table.concat({
    "### Security review step: " .. tostring(slot.id),
    "",
    "Result:",
    "```",
    tostring(output or ""),
    "```",
    "",
    marker_text,
  }, "\n")
end

local function analysis_prompt(step_ctx)
  local slot = step_ctx.slot
  local content = slot.content or {}
  local scope = step_ctx.scope or {}
  return table.concat({
    tostring(content.generator or ""),
    "",
    "Review issue: " .. tostring(scope.repo or "") .. "#" .. tostring(scope.number or ""),
    "Workflow: " .. tostring(step_ctx.workflow_id or ""),
    "Step: " .. tostring(slot.id or ""),
    "Prior step results are posted as comments on the review issue; read them from the thread before deciding.",
  }, "\n")
end

local function existing_outcome(step_ctx)
  local key = step_ctx.child_dedup_key
  for _, fact in ipairs(step_ctx.facts or {}) do
    if type(fact) == "table" and fact.child_dedup == key then
      if fact.state == "created" then
        return "exists"
      end
      if fact.state == "generated" then
        return "wait"
      end
    end
  end
  return nil
end

-- deps = { marker, repo, spawn_analysis, final_step_id, label_available, max_requests }
function M.build(deps)
  local marker = deps.marker
  local repo = deps.repo

  local function spawn(prompt)
    local ok, result = pcall(deps.spawn_analysis, prompt)
    if not ok then
      return nil, "analysis-spawn-failed"
    end
    if type(result) ~= "table" then
      return nil, "analysis-no-result"
    end
    if result.exit_code ~= 0 then
      if tonumber(result.exit_code) == 124 then
        return nil, "analysis-timeout"
      end
      return nil, "analysis-nonzero"
    end
    return result, nil
  end

  local function created_marker(step_ctx, output)
    local slot = step_ctx.slot
    local generated_spec = { title = slot.title, body = output }
    local gen_contract_digest = materialization.generator_contract_digest(slot)
    local gen_spec_digest = materialization.generated_spec_digest(generated_spec)
    if gen_contract_digest == nil or gen_spec_digest == nil then
      return nil, "invalid-step-digest"
    end
    local child_issue = tostring((step_ctx.scope or {}).number or "")
    return marker.build_materialization_marker(
      step_ctx.origin,
      step_ctx.blueprint_digest,
      slot.id,
      step_ctx.predecessor_ref_digest,
      gen_contract_digest,
      gen_spec_digest,
      step_ctx.child_dedup_key,
      child_issue,
      "created"
    )
  end

  local function file_findings(step_ctx, output)
    local ok, findings = pcall(security_logic.decode_findings, output)
    if not ok then
      return nil, "findings-invalid"
    end
    local requests = security_logic.build_finding_requests(repo, findings, deps.label_available, deps.max_requests)
    for _, request in ipairs(requests) do
      raise("github-issue-effect.github_issue_create_request", request)
    end
    return #requests, nil
  end

  local function post_step(step_ctx, output)
    local marker_text, marker_err = created_marker(step_ctx, output)
    if marker_text == nil then
      return nil, marker_err or "invalid-step-marker"
    end
    local scope = step_ctx.scope or {}
    raise("github-comment-effect.github_issue_comment_request", {
      schema = "github-proxy.issue-comment.v1",
      repo = scope.repo or repo,
      issue_number = scope.number,
      body = analysis_comment_body(step_ctx.slot, output, marker_text),
      dedup_key = step_ctx.child_dedup_key,
      source_ref = {
        kind = "workflow-security",
        ref = tostring(step_ctx.origin) .. "/" .. tostring(step_ctx.slot.id),
      },
    })
    return "raised", nil
  end

  local executor = {}

  function executor.raise_step(step_ctx)
    local prior = existing_outcome(step_ctx)
    if prior ~= nil then
      return prior
    end
    local result, reason = spawn(analysis_prompt(step_ctx))
    if result == nil then
      return nil, reason
    end
    local output = tostring(result.stdout or "")
    if step_ctx.slot.id == deps.final_step_id then
      local _count, file_err = file_findings(step_ctx, output)
      if file_err ~= nil then
        return nil, file_err
      end
    end
    return post_step(step_ctx, output)
  end

  function executor.emit_terminal(scope, origin, state, reason_code)
    local terminal_state = tostring(state or "error")
    local marker_text = marker.build_terminal_marker(origin, terminal_state, tostring(reason_code or "terminal"))
    if marker_text == nil then
      return nil
    end
    raise("github-comment-effect.github_issue_comment_request", {
      schema = "github-proxy.issue-comment.v1",
      repo = (scope or {}).repo or repo,
      issue_number = (scope or {}).number,
      body = "Security review " .. terminal_state .. ": " .. tostring(reason_code or "terminal") .. "\n\n" .. marker_text,
      dedup_key = tostring(origin) .. "/terminal/" .. terminal_state,
      source_ref = {
        kind = "workflow-security",
        ref = tostring(origin) .. "/terminal",
      },
    })
    return nil
  end

  return executor
end

return M
