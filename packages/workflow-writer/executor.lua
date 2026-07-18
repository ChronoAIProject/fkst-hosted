-- workflow-writer: the EXECUTOR seam (raise_step / emit_terminal).
--
-- The kernel owns WHEN a step is materialized and the CAS key; this module owns HOW the
-- single authoring step becomes a real artifact: a reviewable pull request that adds a
-- new template file (create mode) or edits a target package's template (refine mode).
-- The codex agent does the file edits + PR open (the gh/git text lives in its prompt,
-- never in this Lua); this executor gates the result on the KERNEL validator before
-- recording success -- a drafted template that fails workflow.engine.blueprint.validate
-- (reused via authoring.validate_drafted_template) or collides with an existing catalog
-- id is refused (fail), so an untrusted request can never land an invalid template.
--
-- raise_step is idempotent on step_ctx.child_dedup_key: a repeat call finds the existing
-- materialization marker and returns "exists"; an in-flight generated marker returns
-- "wait"; otherwise it spawns codex, validates the echoed draft, publishes a "created"
-- marker (carrying the delivered PR number) as a bot comment, and returns "raised".
local materialization = require("workflow.engine.materialization")
local authoring = require("authoring")

local M = {}

local function authoring_comment_body(slot, pr_ref, marker_text)
  return table.concat({
    "### Workflow authoring step: " .. tostring(slot.id),
    "",
    "Delivered pull request: " .. tostring(pr_ref ~= nil and pr_ref or "(see run log)"),
    "",
    marker_text,
  }, "\n")
end

-- Idempotency probe: has this exact child (keyed by child_dedup_key) already been
-- materialized? A "created" fact means the PR exists (exists); a "generated" fact means
-- a create is in flight (wait). Distinct from the security probe by shape and naming.
local function prior_outcome(step_ctx)
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

-- Best-effort parse of the opened PR number from the codex stdout. The marker keys on
-- child_dedup_key for idempotency regardless; the PR number lets the completion reader
-- resolve the PR's live merge state. A miss returns nil (completion then waits).
local function parse_pr_number(stdout)
  local text = tostring(stdout or "")
  local from_url = text:match("/pull/(%d+)")
  if from_url ~= nil then
    return from_url
  end
  return text:match("[Pp][Rr]%s*#?(%d+)")
end

-- deps = { marker, repo, spawn_author, step_id, catalog_ids }
function M.build(deps)
  local marker = deps.marker
  local repo = deps.repo

  local function spawn(prompt)
    local ok, result = pcall(deps.spawn_author, prompt)
    if not ok then
      return nil, "authoring-spawn-failed"
    end
    if type(result) ~= "table" then
      return nil, "authoring-no-result"
    end
    if result.exit_code ~= 0 then
      if tonumber(result.exit_code) == 124 then
        return nil, "authoring-timeout"
      end
      return nil, "authoring-nonzero"
    end
    return result, nil
  end

  -- Reuse the kernel validator over the drafted template the agent echoes on stdout,
  -- then run the id-collision guard. Returns nil,reason on any failure.
  local function gate_draft(step_ctx, routing, output)
    local ok, template_json = pcall(authoring.extract_template_json, output)
    if not ok then
      return nil, "draft-not-json"
    end
    local drafted, why = authoring.validate_drafted_template(template_json)
    if drafted == nil then
      return nil, "draft-invalid"
    end
    local existing = type(deps.catalog_ids) == "function" and deps.catalog_ids() or {}
    if authoring.id_collision(drafted.id, existing, routing.mode, routing.target_workflow_id) then
      return nil, "draft-id-collision"
    end
    return drafted, nil
  end

  local function created_marker(step_ctx, pr_ref)
    local slot = step_ctx.slot
    local generated_spec = { title = slot.title, body = tostring(pr_ref or "pending") }
    local gen_contract_digest = materialization.generator_contract_digest(slot)
    local gen_spec_digest = materialization.generated_spec_digest(generated_spec)
    if gen_contract_digest == nil or gen_spec_digest == nil then
      return nil, "invalid-step-digest"
    end
    return marker.build_materialization_marker(
      step_ctx.origin,
      step_ctx.blueprint_digest,
      slot.id,
      step_ctx.predecessor_ref_digest,
      gen_contract_digest,
      gen_spec_digest,
      step_ctx.child_dedup_key,
      tostring(pr_ref or ""),
      "created"
    )
  end

  local function post_created(step_ctx, pr_ref)
    local marker_text, marker_err = created_marker(step_ctx, pr_ref)
    if marker_text == nil then
      return nil, marker_err or "invalid-step-marker"
    end
    local scope = step_ctx.scope or {}
    raise("github-proxy.github_issue_comment_request", {
      schema = "github-proxy.issue-comment.v1",
      repo = scope.repo or repo,
      issue_number = scope.number,
      body = authoring_comment_body(step_ctx.slot, pr_ref, marker_text),
      dedup_key = step_ctx.child_dedup_key,
      source_ref = {
        kind = "workflow-writer",
        ref = tostring(step_ctx.origin) .. "/" .. tostring(step_ctx.slot.id),
      },
    })
    return "raised", nil
  end

  local executor = {}

  function executor.raise_step(step_ctx)
    local prior = prior_outcome(step_ctx)
    if prior ~= nil then
      return prior
    end
    local routing = authoring.classify_request(step_ctx.scope)
    local prompt = authoring.build_prompt(step_ctx.scope, routing)
    local result, reason = spawn(prompt)
    if result == nil then
      return nil, reason
    end
    local output = tostring(result.stdout or "")
    local _drafted, gate_err = gate_draft(step_ctx, routing, output)
    if gate_err ~= nil then
      return nil, gate_err
    end
    return post_created(step_ctx, parse_pr_number(output))
  end

  function executor.emit_terminal(scope, origin, state, reason_code)
    local terminal_state = tostring(state or "error")
    local marker_text = marker.build_terminal_marker(origin, terminal_state, tostring(reason_code or "terminal"))
    if marker_text == nil then
      return nil
    end
    raise("github-proxy.github_issue_comment_request", {
      schema = "github-proxy.issue-comment.v1",
      repo = (scope or {}).repo or repo,
      issue_number = (scope or {}).number,
      body = "Workflow authoring " .. terminal_state .. ": " .. tostring(reason_code or "terminal") .. "\n\n" .. marker_text,
      dedup_key = tostring(origin) .. "/terminal/" .. terminal_state,
      source_ref = {
        kind = "workflow-writer",
        ref = tostring(origin) .. "/terminal",
      },
    })
    return nil
  end

  return executor
end

return M
