local M = {}

local default_angles = { "minimal", "structural", "delete" }
-- Angle count and per-reply length are capped so consensus_reached has a PROVABLE upper
-- bound. Worst-case raw content = max_angles * max_reply_len = 8000 bytes; even at the
-- JSON worst case of 6 bytes/char (\uXXXX escaping) that is ~48 KiB, which with field
-- overhead stays under the reliable-delivery 64 KiB cap. We cannot measure the encoded
-- size at runtime (the SDK exposes json.decode only), so the bound is enforced statically.
local max_angles = 4
local max_key_len = 200
local max_title_len = 240
local max_body_len = 12000
local max_context_len = 8000
local max_reply_len = 2000

local function trim(value)
  return tostring(value or ""):gsub("^%s+", ""):gsub("%s+$", "")
end

local function is_bounded_string(value, limit)
  return type(value) == "string" and value ~= "" and #value <= limit
end

local function is_path_safe_key(value)
  if not is_bounded_string(value, max_key_len) then
    return false
  end
  if value:sub(1, 1) == "/" then
    return false
  end
  if value:find("\\", 1, true) ~= nil then
    return false
  end
  if value:find("%s") ~= nil then
    return false
  end
  if value:find("[^%w%._%-%/#]") ~= nil then
    return false
  end
  for segment in value:gmatch("[^/]+") do
    if segment == "." or segment == ".." then
      return false
    end
  end
  return true
end

local function has_source_ref(value)
  return type(value) == "table"
    and is_bounded_string(value.kind, max_key_len)
    and is_bounded_string(value.ref, max_key_len)
end

local function normalized_angles(proposal)
  if type(proposal.angles) ~= "table" then
    return default_angles
  end

  local angles = {}
  for _, angle in ipairs(proposal.angles) do
    if not is_bounded_string(angle, max_key_len) then
      return nil
    end
    table.insert(angles, angle)
  end
  if #angles == 0 or #angles > max_angles then
    return nil
  end
  return angles
end

function M.is_eligible(proposal)
  if type(proposal) ~= "table" then
    return false
  end
  if proposal.schema ~= "consensus.proposal.v1" then
    return false
  end
  if not is_path_safe_key(proposal.proposal_id) then
    return false
  end
  if not is_path_safe_key(proposal.dedup_key) then
    return false
  end
  if not has_source_ref(proposal.source_ref) then
    return false
  end
  if not is_bounded_string(proposal.title, max_title_len) then
    return false
  end
  if not is_bounded_string(proposal.body, max_body_len) then
    return false
  end
  if proposal.context ~= nil and not is_bounded_string(proposal.context, max_context_len) then
    return false
  end
  return normalized_angles(proposal) ~= nil
end

function M.angles(proposal)
  return normalized_angles(proposal)
end

function M.render_template(template, vars)
  if type(template) ~= "string" then
    error("consensus: template must be a string")
  end
  if type(vars) ~= "table" then
    error("consensus: template vars must be a table")
  end

  return (template:gsub("{{([%w_]+)}}", function(name)
    local value = vars[name]
    if value == nil then
      error("consensus: missing template var " .. name)
    end
    return tostring(value)
  end))
end

-- Keyed by dedup_key (which versions the proposal), not proposal_id, so an updated
-- proposal re-derives consensus instead of being silently skipped.
function M.reached_cache_key(dedup_key)
  if not is_path_safe_key(dedup_key) then
    error("consensus: invalid dedup_key")
  end
  return "consensus/reached/" .. tostring(dedup_key)
end

function M.build_angle_prompt(proposal, angle)
  if type(proposal) ~= "table" then
    error("consensus: proposal must be a table")
  end
  if not is_bounded_string(angle, max_key_len) then
    error("consensus: angle must be a bounded string")
  end

  -- Instruction lines deliberately do NOT begin with "VERDICT:" / "REPLY:" so that a
  -- model echoing the prompt cannot produce lines the strict parser would mistake for
  -- the real answer.
  local prompt = require("prompts.angle")
  local context_block = ""
  if proposal.context ~= nil and proposal.context ~= "" then
    context_block = "Context:\n" .. tostring(proposal.context)
  end

  return M.render_template(prompt.template, {
    bias = prompt.bias[angle] or ("Bias: " .. tostring(angle) .. ". Judge from this named perspective."),
    angle = angle,
    title = proposal.title,
    body = proposal.body,
    context_block = context_block,
  })
end

-- Fail-closed parse. A genuine answer is an ADJACENT pair: exactly one clean VERDICT line
-- immediately followed by exactly one REPLY line (the prompt asks for line one = VERDICT,
-- line two = REPLY). VERDICT must be one whitelist word on its own line (rejects the prompt
-- echo "VERDICT: approve|reject|abstain", "approve/reject", "approve-ish"); REPLY must be
-- anchored at line start (rejects "NOREPLY:" / "NOT REPLY:"). A proposal body/context is
-- untrusted and may be echoed into stdout, so requiring a UNIQUE ADJACENT pair closes both
-- duplicate injection (a second clean VERDICT/REPLY) and orphan pairing (a lone echoed REPLY
-- attached to a verdict that lacked its own reply). Overlong replies are NOT truncated here;
-- aggregate() rejects them so we never raise a partial body.
function M.parse_angle_output(stdout)
  local text = tostring(stdout or "")

  local verdict = nil
  local verdict_count = 0
  local verdict_index = nil
  local reply = nil
  local reply_count = 0
  local reply_index = nil
  local index = 0
  for line in (text .. "\n"):gmatch("(.-)\n") do
    index = index + 1

    local token = line:match("^%s*[Vv][Ee][Rr][Dd][Ii][Cc][Tt]%s*:%s*(%a+)%s*$")
    if token ~= nil then
      local lowered = token:lower()
      if lowered == "approve" or lowered == "reject" or lowered == "abstain" then
        verdict = lowered
        verdict_count = verdict_count + 1
        verdict_index = index
      end
    end

    local captured = line:match("^%s*[Rr][Ee][Pp][Ll][Yy]%s*:%s*(.+)$")
    if captured ~= nil then
      captured = trim(captured)
      if captured ~= "" then
        reply = captured
        reply_count = reply_count + 1
        reply_index = index
      end
    end
  end

  if verdict_count ~= 1 or reply_count ~= 1 then
    return nil
  end
  if reply_index ~= verdict_index + 1 then
    return nil
  end

  return {
    verdict = verdict,
    reply = reply,
  }
end

function M.aggregate(angle_results)
  if type(angle_results) ~= "table" or #angle_results == 0 then
    return nil
  end

  local decision = nil
  for _, result in ipairs(angle_results) do
    if type(result) ~= "table" or result.exit_code ~= 0 then
      return nil
    end
    if result.verdict ~= "approve" and result.verdict ~= "reject" then
      return nil
    end
    if not is_bounded_string(result.reply, max_reply_len) then
      return nil
    end
    if decision == nil then
      decision = result.verdict
    elseif decision ~= result.verdict then
      return nil
    end
  end

  return decision
end

function M.build_reached_payload(proposal, decision, angle_results)
  if type(proposal) ~= "table" then
    error("consensus: proposal must be a table")
  end
  if decision ~= "approve" and decision ~= "reject" then
    error("consensus: invalid decision")
  end
  if not has_source_ref(proposal.source_ref) then
    error("consensus: missing source_ref")
  end

  -- angle_results carries only {angle, verdict}; the full reply text lives in `body`
  -- exactly once. Duplicating replies in both fields could push consensus_reached past
  -- the reliable 64 KiB payload bound.
  local clean_results = {}
  local body_lines = {}
  for _, result in ipairs(angle_results or {}) do
    table.insert(clean_results, {
      angle = result.angle,
      verdict = result.verdict,
    })
    table.insert(body_lines, tostring(result.angle) .. ":")
    table.insert(body_lines, tostring(result.reply))
    table.insert(body_lines, "")
  end

  if #body_lines > 0 then
    table.remove(body_lines)
  end

  return {
    schema = "consensus.consensus_reached.v1",
    proposal_id = proposal.proposal_id,
    decision = decision,
    body = table.concat(body_lines, "\n"),
    angle_results = clean_results,
    dedup_key = "consensus:" .. tostring(proposal.dedup_key),
    -- Normalize to {kind, ref} only: passing the input table through would let an
    -- upstream add unbounded extra fields that could push the payload past 64 KiB.
    source_ref = {
      kind = proposal.source_ref.kind,
      ref = proposal.source_ref.ref,
    },
  }
end

return M
