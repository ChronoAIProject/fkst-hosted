local core = require("core")

local M = {}

M.spec = {
  consumes = { "proposal" },
  produces = { "consensus_reached", "consensus_converge" },
  stall_window = "2m",
}

local function spawn_angle(proposal, angle)
  return spawn_codex({
    prompt = core.build_angle_prompt(proposal, angle),
  })
end

local function spawn_meta_judge(proposal, angle_results)
  return spawn_codex_sync({
    prompt = core.build_meta_judge_prompt(proposal, angle_results),
  })
end

local function raise_converge(proposal, angle_results, narrowed_question)
  raise(
    "consensus_converge",
    core.build_converge_payload(proposal, narrowed_question, angle_results)
  )
end

function pipeline(event)
  local proposal = event.payload or {}
  if proposal.schema ~= "consensus.proposal.v1" then
    log.warn("consensus: unsupported proposal schema")
    return
  end
  if not core.is_eligible(proposal) then
    return
  end

  local cache_key = core.reached_cache_key(proposal.dedup_key)
  with_lock(cache_key, function()
    if cache_get(cache_key) then
      return
    end

    local angle_results = {}
    local handles = {}
    local angles = core.angles(proposal)
    local verdict_mode = core.verdict_mode(proposal)
    for _, angle in ipairs(angles) do
      table.insert(handles, spawn_angle(proposal, angle))
    end

    local results = await_all(handles)
    for index, angle in ipairs(angles) do
      local parsed = nil
      local result = results[index]
      if type(result) == "table" and result.exit_code == 0 then
        parsed = core.parse_angle_output(result.stdout, verdict_mode)
      end
      table.insert(angle_results, {
        angle = angle,
        verdict = parsed and parsed.verdict or nil,
        reply = parsed and parsed.reply or nil,
        stdout = type(result) == "table" and result.stdout or nil,
        exit_code = type(result) == "table" and result.exit_code or nil,
      })
    end

    local decision = core.aggregate(angle_results, verdict_mode)
    if decision ~= nil then
      raise("consensus_reached", core.build_reached_payload(proposal, decision, angle_results))
      cache_set(cache_key, proposal.dedup_key)
      return
    end

    local meta_result = spawn_meta_judge(proposal, angle_results)
    local parsed = nil
    if type(meta_result) == "table" and meta_result.exit_code == 0 then
      parsed = core.parse_meta_judge_output(meta_result.stdout, verdict_mode)
    end
    if parsed ~= nil and parsed.kind == "reached" then
      raise("consensus_reached", core.build_reached_payload(
        proposal,
        parsed.decision,
        angle_results,
        parsed.framing
      ))
      cache_set(cache_key, proposal.dedup_key)
      return
    end
    if parsed ~= nil and parsed.kind == "converge" then
      raise_converge(proposal, angle_results, parsed.narrowed_question)
      return
    end

    raise_converge(proposal, angle_results, core.default_narrowed_question(proposal, angle_results))
  end)
end

return M
