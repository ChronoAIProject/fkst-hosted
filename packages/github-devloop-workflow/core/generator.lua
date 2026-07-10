local blueprint = require("core.blueprint")
local workflow_codex = require("workflow.codex")

local M = {}

M.MAX_GENERATED_TITLE_BYTES = blueprint.MAX_STEP_TITLE_BYTES
M.MAX_GENERATED_BODY_BYTES = blueprint.MAX_STATIC_INTENT_BYTES
M.WORKFLOW_GENERATOR_LABEL = "FKST_WORKFLOW_GENERATED_ISSUE_V1"

local function trim(value)
  return (tostring(value or ""):gsub("^%s+", ""):gsub("%s+$", ""))
end

local function bounded_nonempty(value, limit)
  return type(value) == "string" and value ~= "" and #value <= limit
end

local function validate_spec(spec)
  if type(spec) ~= "table" then
    return nil, "invalid-generator-output"
  end
  local title = trim(spec.title)
  local body = trim(spec.body)
  if not bounded_nonempty(title, M.MAX_GENERATED_TITLE_BYTES) then
    return nil, "invalid-title"
  end
  if not bounded_nonempty(body, M.MAX_GENERATED_BODY_BYTES) then
    return nil, "invalid-body"
  end
  return {
    title = title,
    body = body,
  }, nil
end

local function static_spec(slot)
  local content = slot.content or {}
  return validate_spec({
    title = slot.title,
    body = content.intent,
  })
end

local function source_ref_text(source_ref)
  if type(source_ref) ~= "table" then
    return "(none)"
  end
  return tostring(source_ref.kind or "") .. ":" .. tostring(source_ref.ref or "")
end

local function build_generated_prompt(ctx, slot, predecessor_result_ref, content_fetch_ref)
  local content = slot.content or {}
  local source_ref = predecessor_result_ref and predecessor_result_ref.source_ref
    or predecessor_result_ref
  local fetch_clause = ""
  if content_fetch_ref ~= nil and tostring(content_fetch_ref) ~= "" then
    fetch_clause = "\nOptional local predecessor context (best-effort snapshot; if absent or unreadable, fetch from the source_ref above):\n" .. tostring(content_fetch_ref)
  end
  return table.concat({
    "You are generating exactly one GitHub issue spec for a bounded fkst workflow slot.",
    "Return exactly one JSON object with string fields title and body. No markdown fence.",
    "Do not invent additional slots, branches, loops, labels, or workflow state.",
    "The predecessor result content is not embedded in this prompt. Fetch the full predecessor result from the source_ref before deciding.",
    "Origin: " .. tostring(ctx and ctx.origin_proposal_id or ""),
    "Workflow: " .. tostring(ctx and ctx.workflow_id or ""),
    "Slot: " .. tostring(slot.id or ""),
    "Predecessor source_ref: " .. source_ref_text(source_ref),
    fetch_clause,
    "Generator instruction:",
    tostring(content.generator or ""),
  }, "\n")
end

local function parse_labeled_text(stdout)
  local text = tostring(stdout or "")
  local title = text:match("TITLE:%s*(.-)\n")
  local body = text:match("BODY:%s*(.+)$")
  if title == nil or body == nil then
    return nil
  end
  return {
    title = trim(title),
    body = trim(body),
  }
end

local function parse_generator_stdout(stdout)
  local ok, decoded = pcall(json.decode, tostring(stdout or ""))
  if ok and type(decoded) == "table" then
    return validate_spec(decoded)
  end
  local labeled = parse_labeled_text(stdout)
  if labeled ~= nil then
    return validate_spec(labeled)
  end
  return nil, "invalid-generator-output"
end

local function spawn_generated(deps, prompt, ctx)
  if type(deps.spawn_codex) == "function" then
    return deps.spawn_codex(prompt, ctx)
  end
  if type(deps.spawn_codex_sync) == "function" then
    return deps.spawn_codex_sync(workflow_codex.unrestricted_codex_opts(prompt, ctx and ctx.worktree))
  end
  return nil, "missing-generator-runner"
end

local function generated_spec(deps, ctx, slot, predecessor_result_ref)
  if predecessor_result_ref == nil then
    return nil, "missing-predecessor-result"
  end
  -- The pre-fetch is a best-effort OPTIMIZATION: it hands the codex a local
  -- snapshot of the predecessor result. It is NOT the dependency — the codex
  -- already has the predecessor source_ref (+ full access) and is instructed to
  -- fetch it directly. So a pre-fetch failure (e.g. an unavailable devloop board
  -- in one-shot run) must NOT block materialization; we fall back to source_ref.
  local content_fetch_ref = nil
  if type(deps.content_fetch) == "function" then
    local ok, value = pcall(deps.content_fetch, predecessor_result_ref, ctx)
    if ok then
      content_fetch_ref = value
    end
  end

  local prompt = build_generated_prompt(ctx or {}, slot, predecessor_result_ref, content_fetch_ref)
  local ok, result, spawn_reason = pcall(spawn_generated, deps, prompt, ctx or {})
  if not ok or type(result) ~= "table" or result.exit_code ~= 0 then
    return nil, spawn_reason or "generator-codex-failed"
  end
  return parse_generator_stdout(result.stdout)
end

function M.run_slot_generator(deps, ctx, slot, predecessor_result_ref)
  if type(slot) ~= "table" or type(slot.content) ~= "table" then
    return nil, "invalid-slot"
  end
  local kind = slot.content.kind
  if kind == "static" then
    return static_spec(slot)
  end
  if kind == "generated" then
    return generated_spec(deps or {}, ctx or {}, slot, predecessor_result_ref)
  end
  return nil, "unsupported-content-kind"
end

M.build_generated_prompt = build_generated_prompt
M.parse_generator_stdout = parse_generator_stdout

function M.install(target)
  target.generator = M
  target.run_slot_generator = M.run_slot_generator
end

return M
