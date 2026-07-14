local core = require("core")
local angle_answers = require("angle_answers")
local convergence_identity = require("contract.convergence_identity")
local rebuttal = require("departments.decide.rebuttal")
local result_memo = require("departments.decide.result_memo")
local synthesis = require("departments.decide.synthesis")
local workflow_codex = require("workflow.codex")
local saga = require("workflow.saga")

local aggregate = core.aggregate
local build_reached_payload = core.build_reached_payload
local judgment_scratch_worktree = core.judgment_scratch_worktree
local parse_angle_output = core.parse_angle_output
local result_memo_key = core.result_memo_key

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

local function checkout_root_exists(path)
  local result = exec_sync({ cmd = core.checkout_root_exists_cmd(path), timeout = 30 })
  return result.exit_code == 0
end

local function prepare_seat_worktree(proposal, scratch_path)
  if proposal.worktree == nil then
    return prepare_judgment_worktree(scratch_path)
  end
  if checkout_root_exists(proposal.worktree) then
    return proposal.worktree
  end
  if checkout_root_exists(".") then
    return "."
  end
  error("consensus: judgment-worktree-unavailable: proposal checkout and fallback checkout are unavailable")
end

local function codex_opts(proposal, prompt, worktree, role)
  return core.judgment_codex_opts(prompt, worktree)
end

local function codex_identity(proposal, role, angle_lane)
  return convergence_identity.from_proposal(role or "consensus", proposal, {
    angle_lane = angle_lane,
  })
end

local function defer_live_run(identity)
  error(
    "consensus: live-run-active: role=" .. tostring(identity.role)
      .. " proposal_id=" .. tostring(identity.proposal_id)
      .. " dedup_key=" .. tostring(identity.dedup_key)
  )
end

local function dispatch_codex(proposal, prompt, worktree, role, angle_lane, opts)
  local run_identity = codex_identity(proposal, role, angle_lane)
  local dispatch_opts = codex_opts(proposal, prompt, worktree, run_identity.role)
  for key, value in pairs(opts or {}) do
    dispatch_opts[key] = value
  end
  local result = workflow_codex.dispatch(run_identity, dispatch_opts)
  if type(result) == "table" and result.deferred then
    defer_live_run(run_identity)
  end
  return result
end

local function spawn_angle(proposal, angle, runtime_root)
  local prompt = core.build_angle_prompt(proposal, angle)
  local worktree = prepare_seat_worktree(proposal,
    judgment_scratch_worktree(runtime_root, "angle-" .. tostring(angle), proposal.dedup_key)
  )
  return dispatch_codex(proposal, prompt, worktree, "consensus", tostring(angle))
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
  local angle_results = {}
  local handles = {}
  local angles = core.angles(proposal)
  local verdict_mode = core.verdict_mode(proposal)
  for _, angle in ipairs(angles) do
    local run_identity = codex_identity(proposal, "consensus", tostring(angle))
    if workflow_codex.live_run_active(run_identity) then
      defer_live_run(run_identity)
    end
  end

  local runtime_root = read_runtime_root()
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
      stderr = type(result) == "table" and result.stderr or nil,
      exit_code = type(result) == "table" and result.exit_code or nil,
    })
  end

  angle_answers.assert_all_valid(angle_results, "blind")

  local decision = aggregate(angle_results, verdict_mode)
  if decision ~= nil then
    return {
      queue = "consensus_reached",
      payload = build_reached_payload(proposal, decision, angle_results),
    }
  end

  local rebuttal_results = angle_results
  if rebuttal.can_run(angle_results) then
    local rebuttal_handles = rebuttal.spawn_all({
      proposal = proposal,
      angle_results = angle_results,
      runtime_root = runtime_root,
      prepare_judgment_worktree = function(path)
        return prepare_seat_worktree(proposal, path)
      end,
      codex_opts = codex_opts,
      build_rebuttal_prompt = function(target_proposal, own_result, peer_results)
        return core.build_rebuttal_prompt(target_proposal, own_result, peer_results)
      end,
      judgment_scratch_worktree = function(root, kind, identity)
        return judgment_scratch_worktree(root, kind, identity)
      end,
      dispatch_codex = function(target_proposal, prompt, worktree, role, angle_lane)
        return dispatch_codex(target_proposal, prompt, worktree, role, angle_lane)
      end,
    })
    local rebuttal_outputs = await_all(rebuttal_handles)
    rebuttal_results = rebuttal.collect(angle_results, rebuttal_outputs, verdict_mode, {
      parse_angle_output = function(stdout, mode)
        return parse_angle_output(stdout, mode)
      end,
    })
    angle_answers.assert_all_valid(rebuttal_results, "rebuttal")
    local rebuttal_reached = rebuttal.post_rebuttal_reached(proposal, angle_results, rebuttal_results, verdict_mode, {
      aggregate = function(items, mode)
        return aggregate(items, mode)
      end,
      assert_all_angle_answers_valid = function(results, phase)
        return angle_answers.assert_all_valid(results, phase)
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
      local worktree = prepare_seat_worktree(proposal,
        judgment_scratch_worktree(runtime_root, repair and "synthesis-repair" or "synthesis", proposal.dedup_key)
      )
      return dispatch_codex(proposal, prompt, worktree, "consensus", repair and "synthesis-repair" or "synthesis", {
        sync = true,
      })
    end,
  })
  return synthesis.to_decision_result(proposal, angle_results, rebuttal_results, parsed, {
    assert_all_angle_answers_valid = function(results, phase)
      return angle_answers.assert_all_valid(results, phase)
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
  return false
end

local function act_decide(event)
  local proposal = event.payload or {}
  local cache_key = result_memo_key(proposal.dedup_key)
  local memoized_payload = result_memo.load(cache_key, proposal.dedup_key)
  if memoized_payload ~= nil then
    raise("consensus_reached", memoized_payload)
    return
  end

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
    memoized_payload = result_memo.load(cache_key, proposal.dedup_key)
    if memoized_payload == nil and result.queue == "consensus_reached" then
      result_memo.save(cache_key, result.payload)
      memoized_payload = result.payload
    end
  end)
  if memoized_payload ~= nil then
    raise("consensus_reached", memoized_payload)
    return
  end
  if result.queue == "consensus_converge" then
    raise_converge(proposal, result.angle_results, result.narrowed_question, result.findings_record, result.essence_stall)
    return
  end
  error("consensus: decision-result-invalid: unknown decision result")
end

return saga.department(spec, {
  done = decision_done,
  act = act_decide,
  wrap = core.wrap_pipeline_failure,
  name = "decide",
})
