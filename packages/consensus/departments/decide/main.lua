local core = require("core")

local M = {}

M.spec = {
  consumes = { "proposal" },
  produces = { "consensus_reached", "consensus_unresolved" },
  stall_window = "2m",
}

local function run_angle(proposal, angle)
  local result = spawn_codex_sync({
    prompt = core.build_angle_prompt(proposal, angle),
    stall_window = M.spec.stall_window,
  })

  local parsed = nil
  if type(result) == "table" and result.exit_code == 0 then
    parsed = core.parse_angle_output(result.stdout)
  end

  return {
    angle = angle,
    verdict = parsed and parsed.verdict or nil,
    reply = parsed and parsed.reply or nil,
    exit_code = type(result) == "table" and result.exit_code or nil,
  }
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
    -- Future optimization: use spawn_codex plus await_all to parallelize angles.
    for _, angle in ipairs(core.angles(proposal)) do
      table.insert(angle_results, run_angle(proposal, angle))
    end

    local decision = core.aggregate(angle_results)
    if decision == nil then
      raise("consensus_unresolved", core.build_unresolved_payload(proposal))
      return
    end

    raise("consensus_reached", core.build_reached_payload(proposal, decision, angle_results))
    cache_set(cache_key, proposal.dedup_key)
  end)
end

return M
