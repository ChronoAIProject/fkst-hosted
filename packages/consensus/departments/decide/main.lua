local core = require("core")
local rebuttal = require("departments.decide.rebuttal")
local synthesis = require("departments.decide.synthesis")
local saga = require("workflow.saga")

local aggregate = core.aggregate
local build_reached_payload = core.build_reached_payload
local judgment_scratch_worktree = core.judgment_scratch_worktree
local parse_angle_output = core.parse_angle_output
local reached_cache_key = core.reached_cache_key

local spec = {
  consumes = { "proposal" },
  published_seam = { "proposal" },
  produces = { "consensus_reached", "consensus_converge" },
  stall_window = "2m",
}

local function read_runtime_root()
  local result = exec_sync({ cmd = core.read_runtime_root_cmd(), timeout = 30 })
  if result.exit_code ~= 0 then
    error("consensus: runtime-root-read-failed: FKST_RUNTIME_ROOT read failed: " .. tostring(result.stderr))
  end
  return result.stdout
end

local function prepare_judgment_worktree(path)
  local result = exec_sync({ cmd = core.mkdir_p_cmd(path), timeout = 30 })
  if result.exit_code ~= 0 then
    error("consensus: scratch-directory-setup-failed: judgment scratch directory setup failed: " .. tostring(result.stderr))
  end
  return path
end

local function codex_opts(proposal, prompt, worktree, role)
  local opts = core.judgment_codex_opts(prompt, worktree)
  opts.role = role or "consensus"
  opts.proposal_id = proposal.proposal_id
  opts.dedup_key = proposal.dedup_key
  return opts
end

local function spawn_angle(proposal, angle, runtime_root)
  local prompt = core.build_angle_prompt(proposal, angle)
  local worktree = prepare_judgment_worktree(
    judgment_scratch_worktree(runtime_root, "angle-" .. tostring(angle), proposal.dedup_key)
  )
  return spawn_codex(codex_opts(proposal, prompt, worktree, "consensus"))
end

local function raise_converge(proposal, angle_results, narrowed_question, findings_record, essence_stall)
  raise(
    "consensus_converge",
    core.build_converge_payload(proposal, narrowed_question, angle_results, findings_record, {
      essence_stall = essence_stall,
    })
  )
end

local function decide(proposal)
  local runtime_root = read_runtime_root()

  local angle_results = {}
  local handles = {}
  local angles = core.angles(proposal)
  local verdict_mode = core.verdict_mode(proposal)
  for _, angle in ipairs(angles) do
    table.insert(handles, spawn_angle(proposal, angle, runtime_root))
  end

  local results = await_all(handles)
  for index, angle in ipairs(angles) do
    local parsed = nil
    local result = results[index]
    if type(result) == "table" and result.exit_code == 0 then
      parsed = parse_angle_output(result.stdout, verdict_mode)
    end
    table.insert(angle_results, {
      angle = angle,
      verdict = parsed and parsed.verdict or nil,
      reply = parsed and parsed.reply or nil,
      blocking_gap = parsed and parsed.blocking_gap or nil,
      stdout = type(result) == "table" and result.stdout or nil,
      exit_code = type(result) == "table" and result.exit_code or nil,
    })
  end

  local decision = aggregate(angle_results, verdict_mode)
  if decision ~= nil then
    return {
      queue = "consensus_reached",
      payload = build_reached_payload(proposal, decision, angle_results),
      cache = true,
    }
  end

  local rebuttal_results = angle_results
  if rebuttal.can_run(angle_results) then
    local rebuttal_handles = rebuttal.spawn_all({
      proposal = proposal,
      angle_results = angle_results,
      runtime_root = runtime_root,
      prepare_judgment_worktree = prepare_judgment_worktree,
      codex_opts = codex_opts,
      build_rebuttal_prompt = function(target_proposal, own_result, peer_results)
        return core.build_rebuttal_prompt(target_proposal, own_result, peer_results)
      end,
      judgment_scratch_worktree = function(root, kind, identity)
        return judgment_scratch_worktree(root, kind, identity)
      end,
      spawn_codex = spawn_codex,
    })
    local rebuttal_outputs = await_all(rebuttal_handles)
    rebuttal_results = rebuttal.collect(angle_results, rebuttal_outputs, verdict_mode, {
      parse_angle_output = function(stdout, mode)
        return parse_angle_output(stdout, mode)
      end,
    })
    local rebuttal_reached = rebuttal.post_rebuttal_reached(proposal, angle_results, rebuttal_results, verdict_mode, {
      aggregate = function(items, mode)
        return aggregate(items, mode)
      end,
      build_reached_payload = function(target_proposal, decision, results, framing, provenance)
        return build_reached_payload(target_proposal, decision, results, framing, provenance)
      end,
    })
    if rebuttal_reached ~= nil then
      return rebuttal_reached
    end
  end

  local parsed = synthesis.parse_or_retry({
    verdict_mode = verdict_mode,
    p1_results = angle_results,
    p2_results = rebuttal_results,
    build_prompt = function(repair, prior_result)
      return core.build_synthesis_prompt(proposal, angle_results, rebuttal_results, {
        repair = repair,
        prior_result = prior_result,
      })
    end,
    spawn_sync = function(_kind, prompt)
      local repair = _kind == "synthesis-repair"
      local worktree = prepare_judgment_worktree(
        judgment_scratch_worktree(runtime_root, repair and "synthesis-repair" or "synthesis", proposal.dedup_key)
      )
      return spawn_codex_sync(codex_opts(proposal, prompt, worktree, "consensus"))
    end,
  })
  return synthesis.to_decision_result(proposal, angle_results, rebuttal_results, parsed, {
    all_angles_succeeded = function(results)
      return core.all_angles_succeeded(results)
    end,
    build_reached_payload = function(target_proposal, decision, results, framing, provenance)
      return build_reached_payload(target_proposal, decision, results, framing, provenance)
    end,
  })
end

local function decision_done(event)
  local proposal = event.payload or {}
  if proposal.schema ~= "consensus.proposal.v1" then
    log.warn("consensus: unsupported proposal schema")
    return true
  end
  if not core.is_eligible(proposal) then
    return true
  end

  local cache_key = reached_cache_key(proposal.dedup_key)
  local already_reached = false
  with_lock(cache_key, function()
    already_reached = cache_get(cache_key) ~= nil
  end)
  return already_reached
end

local function act_decide(event)
  local proposal = event.payload or {}
  local cache_key = reached_cache_key(proposal.dedup_key)

  local ok, result = pcall(decide, proposal)
  if not ok then
    if core.is_stale_generation_context_error(result) then
      log.warn(
        "consensus dept=decide tag=STALE_GENERATION_CONTEXT"
          .. " proposal_id=" .. tostring(proposal.proposal_id)
          .. " dedup_key=" .. tostring(proposal.dedup_key)
          .. " error_class=" .. core.stale_generation_context_error_class()
      )
      return
    end
    error(result)
  end

  with_lock(cache_key, function()
    if cache_get(cache_key) then
      return
    end
    if result.queue == "consensus_reached" then
      raise("consensus_reached", result.payload)
      if result.cache then
        cache_set(cache_key, proposal.dedup_key)
      end
      return
    end
    if result.queue == "consensus_converge" then
      raise_converge(proposal, result.angle_results, result.narrowed_question, result.findings_record, result.essence_stall)
      return
    end
    error("consensus: decision-result-invalid: unknown decision result")
  end)
end

return saga.department(spec, {
  done = decision_done,
  act = act_decide,
  wrap = core.wrap_pipeline_failure,
  name = "decide",
})
