local M = {}
local provenance = require("departments.decide.provenance")

local function unanimous_verdict(results)
  local first = nil
  for _, item in ipairs(results or {}) do
    if type(item) ~= "table" or item.verdict == nil then
      return nil
    end
    if first == nil then
      first = item.verdict
    elseif item.verdict ~= first then
      return nil
    end
  end
  return first
end

function M.parse_stance(stdout, stance_label)
  local text = tostring(stdout or "")
  local parsed = nil
  local count = 0
  local label = stance_label or "⟦FKST:STANCE⟧"
  for line in (text .. "\n"):gmatch("(.-)\n") do
    local stance, rest = line:match("^%s*" .. label .. "%s*(%a+)%s*(.-)%s*$")
    if stance ~= nil then
      stance = stance:lower()
      if stance ~= "update" and stance ~= "defend" then
        return nil
      end
      count = count + 1
      if stance == "update" then
        local peer_claim = tostring(rest:match("[Bb][Ee][Cc][Aa][Uu][Ss][Ee]%s+(.+)$") or ""):match("^%s*(.-)%s*$")
        if peer_claim == "" then
          return nil
        end
        parsed = {
          stance = stance,
          peer_claim = peer_claim,
        }
      else
        parsed = {
          stance = stance,
        }
      end
    end
  end
  if count ~= 1 then
    return nil
  end
  return parsed
end

function M.parse_output(stdout, verdict_mode, caps)
  local stance = M.parse_stance(stdout, caps and caps.stance_label)
  if stance == nil then
    return nil
  end
  local verdict = caps.parse_angle_output(stdout, verdict_mode)
  if verdict == nil then
    return nil
  end
  verdict.stance = stance.stance
  verdict.peer_claim = stance.peer_claim
  return verdict
end

function M.can_run(angle_results)
  -- Rebuttal (Phase R) is meaningful whenever at least two seats can clash; the
  -- spawn/collect machinery is generic over N seats, so admission derives from the
  -- actual seat count rather than a fixed number.
  return type(angle_results) == "table" and #angle_results >= 2
end

local function peer_results(angle_results, own_index)
  local peers = {}
  for index, item in ipairs(angle_results or {}) do
    if index ~= own_index then
      table.insert(peers, item)
    end
  end
  return peers
end

function M.spawn_all(ctx)
  local handles = {}
  local angle_results = ctx.angle_results or {}
  for index, result in ipairs(angle_results) do
    local prompt = ctx.build_rebuttal_prompt(ctx.proposal, result, peer_results(angle_results, index))
    local worktree = ctx.prepare_judgment_worktree(
      ctx.judgment_scratch_worktree(ctx.runtime_root, "rebuttal-" .. tostring(result.angle), ctx.proposal.dedup_key)
    )
    table.insert(handles, ctx.dispatch_codex(ctx.proposal, prompt, worktree, "consensus", "rebuttal-" .. tostring(result.angle)))
  end
  return handles
end

function M.collect(angle_results, results, verdict_mode, caps)
  local rebuttal_results = {}
  for index, angle_result in ipairs(angle_results or {}) do
    local result = results[index]
    local parsed = nil
    if type(result) == "table" and result.exit_code == 0 then
      parsed = M.parse_output(result.stdout, verdict_mode, caps)
    end
    table.insert(rebuttal_results, {
      angle = angle_result.angle,
      verdict = parsed and parsed.verdict or nil,
      reply = parsed and parsed.reply or nil,
      blocking_gap = parsed and parsed.blocking_gap or nil,
      stance = parsed and parsed.stance or nil,
      peer_claim = parsed and parsed.peer_claim or nil,
      stdout = type(result) == "table" and result.stdout or nil,
      exit_code = type(result) == "table" and result.exit_code or nil,
    })
  end
  return rebuttal_results
end

function M.post_rebuttal_reached(proposal, p1_results, p2_results, verdict_mode, caps)
  if unanimous_verdict(p2_results) == nil then
    return nil
  end
  local decision = caps.aggregate(p2_results, verdict_mode)
  if decision == nil then
    return nil
  end
  return {
    queue = "consensus_reached",
    payload = caps.build_reached_payload(proposal, decision, p2_results, nil, {
      verdict_path = "post-rebuttal-unanimity",
      p1_verdicts = provenance.verdict_vector(p1_results),
      p2_verdicts = provenance.verdict_vector(p2_results),
      verified_moves = 0,
    }),
    cache = true,
  }
end

return M
