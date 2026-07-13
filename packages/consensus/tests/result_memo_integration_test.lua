local core = require("core")
local result_memo = require("departments.decide.result_memo")
local t = fkst.test

local old_pipeline = pipeline
local decide_department = require("departments.decide.main")
pipeline = old_pipeline

local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"
local gap_label = "⟦FKST:GAP⟧"
local stance_label = "⟦FKST:STANCE⟧"
local angles = { "teleology", "parsimony", "fidelity" }

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function runtime_root(name)
  return "/tmp/fkst-packages-test/consensus-result-memo/" .. tostring(now()) .. "/" .. nonce() .. "/" .. name
end

local function opts(name)
  return {
    env = {
      FKST_RUNTIME_ROOT = runtime_root(name),
    },
  }
end

local function proposal(extra)
  local value = {
    schema = "consensus.proposal.v1",
    proposal_id = "proposal-42",
    title = "Adopt consensus package",
    body = "Create a small flat package that asks several angles to judge a proposal.",
    content_fetch = "fetch-source --ref demo/consensus/42 --full",
    context = "The package must stay silent unless all angles agree.",
    angles = angles,
    dedup_key = "proposal-42-v1",
    source_ref = {
      kind = "proposal",
      ref = "demo/consensus/42",
    },
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function namespaced_event(payload)
  return {
    queue = "consensus.proposal",
    payload = payload,
  }
end

local function run_namespaced_decide(payload, run_opts)
  return t.run_department(
    "departments/decide/main.lua",
    namespaced_event(payload),
    run_opts
  )
end

local function seed_cache(key, value, run_opts)
  return t.run_department("departments/test_cache_seed/main.lua", {
    queue = "cache_seed",
    payload = {
      key = key,
      value = value,
    },
  }, run_opts)
end

local function mock_judgment_runtime()
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/consensus-result-memo/runtime",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_angle(angle, verdict, reply)
  t.mock_command("mkdir -p", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("consensus-angle-" .. tostring(angle), {
    stdout = verdict_label .. " " .. verdict
      .. "\n" .. reply_label .. " " .. reply
      .. (verdict == "reject" and "\n" .. gap_label .. " " .. reply or "")
      .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_unanimous(verdict, prefix)
  mock_judgment_runtime()
  for _, angle in ipairs(angles) do
    mock_angle(angle, verdict, prefix .. " " .. angle .. ".")
  end
end

local function mock_converge(prefix)
  mock_judgment_runtime()
  for _, angle in ipairs(angles) do
    mock_angle(angle, "abstain", prefix .. " " .. angle .. ".")
  end
  for _, angle in ipairs(angles) do
    t.mock_command("mkdir -p", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("consensus-rebuttal-" .. tostring(angle), {
      stdout = stance_label .. " defend"
        .. "\n" .. verdict_label .. " abstain"
        .. "\n" .. reply_label .. " " .. prefix .. " rebuttal " .. angle .. ".\n",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command("mkdir -p", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("consensus-synthesis-proposal", {
    stdout = "converge: concurrent loser remains unresolved + inspect the loser evidence"
      .. "\nopen: inspect the concurrent loser evidence\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_gate_synthesis_reject(gap)
  mock_judgment_runtime()
  mock_angle("teleology", "comment", "Teleology advisory.")
  mock_angle("parsimony", "abstain", "Parsimony cannot decide.")
  mock_angle("fidelity", "comment", "Fidelity advisory.")

  local rebuttals = {
    teleology = verdict_label .. " reject\n" .. reply_label .. " The diff lacks coverage.\n" .. gap_label .. " " .. gap,
    parsimony = verdict_label .. " comment\n" .. reply_label .. " Keep the patch narrow.",
    fidelity = verdict_label .. " abstain\n" .. reply_label .. " No additional finding.",
  }
  for _, angle in ipairs(angles) do
    t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
    t.mock_command("consensus-rebuttal-" .. angle, {
      stdout = stance_label .. " defend\n" .. rebuttals[angle] .. "\n",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("consensus-synthesis-proposal", {
    stdout = "reached:reject reject until the named gap is fixed\n" .. gap_label .. " " .. gap .. "\n",
    stderr = "",
    exit_code = 0,
  })
end

local function codex_calls()
  local calls = {}
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find("codex exec", 1, true) ~= nil then
      table.insert(calls, call)
    end
  end
  return calls
end

local function with_global_overrides(overrides, fn)
  local originals = {}
  for name, replacement in pairs(overrides) do
    originals[name] = _G[name]
    _G[name] = replacement
  end
  local ok, result = pcall(fn)
  for name, original in pairs(originals) do
    _G[name] = original
  end
  if not ok then
    error(result)
  end
  return result
end

local function without_live_codex_runs(fn)
  local original = fkst.codex_runs
  fkst.codex_runs = function()
    return { running = {}, recent = {} }
  end
  local ok, result = pcall(fn)
  fkst.codex_runs = original
  if not ok then
    error(result)
  end
  return result
end

return {
  test_same_dedup_key_replays_identical_result_memo = function()
    local run_opts = opts("memo-replay")
    mock_unanimous("approve", "Winner approves")

    local first = run_namespaced_decide(proposal(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)
    local first_payload = result_memo.encode(first.raises[1].payload)

    local second = run_namespaced_decide(proposal(), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 1)
    t.eq(second.raises[1].queue, "consensus_reached")
    t.eq(second.raises[1].payload.dedup_key, "consensus:proposal-42-v1")
    t.eq(result_memo.encode(second.raises[1].payload), first_payload)
    t.eq(#codex_calls(), 3)
  end,

  test_same_dedup_key_redelivery_without_memo_recomputes_and_emits = function()
    mock_unanimous("approve", "Winner approves")

    local first = run_namespaced_decide(proposal(), opts("memo-before-loss"))
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)

    mock_unanimous("approve", "After cache loss approves")

    local second = run_namespaced_decide(proposal(), opts("memo-after-loss"))
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 1)
    t.eq(second.raises[1].queue, "consensus_reached")
    t.is_true(second.raises[1].payload.body:find("After cache loss", 1, true) ~= nil)
    t.eq(#codex_calls(), 6)
  end,

  test_same_decision_dedup_key_replays_memoized_effect_version = function()
    local run_opts = opts("effect-version-refire")
    mock_unanimous("approve", "Winner approves")

    local first = run_namespaced_decide(proposal({
      dedup_key = "proposal-42/intake/1234567890",
      effect_version = "intake/proposal-42/2026-06-03T01-02-03Z",
    }), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)
    t.eq(first.raises[1].payload.dedup_key, "consensus:proposal-42/intake/1234567890")
    t.eq(first.raises[1].payload.effect_version, "intake/proposal-42/2026-06-03T01-02-03Z")

    local second = run_namespaced_decide(proposal({
      dedup_key = "proposal-42/intake/1234567890",
      effect_version = "intake/proposal-42/2026-06-03T01-22-03Z",
    }), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 1)
    t.eq(second.raises[1].payload.dedup_key, "consensus:proposal-42/intake/1234567890")
    t.eq(second.raises[1].payload.effect_version, "intake/proposal-42/2026-06-03T01-02-03Z")
    t.eq(#codex_calls(), 3)
  end,

  test_new_version_reruns_consensus = function()
    local run_opts = opts("new-version")
    mock_unanimous("approve", "First version approves")

    local first = run_namespaced_decide(proposal(), run_opts)
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)
    t.eq(first.raises[1].payload.dedup_key, "consensus:proposal-42-v1")

    mock_unanimous("approve", "Second version approves")

    local second = run_namespaced_decide(proposal({ dedup_key = "proposal-42-v2" }), run_opts)
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 1)
    t.eq(second.raises[1].payload.dedup_key, "consensus:proposal-42-v2")
    t.eq(#codex_calls(), 6)
  end,

  test_old_gapless_memo_is_ignored_and_v2_bounded_reject_replays = function()
    local run_opts = opts("memo-contract-v2")
    local target = proposal({
      verdict_mode = "gate",
      dedup_key = "proposal-42-v1/gate-reject-contract",
    })
    local old_key = "consensus/result-memo/" .. target.dedup_key
    local new_key = core.result_memo_key(target.dedup_key)
    t.eq(new_key, "consensus/result-memo/v2/" .. target.dedup_key)
    t.is_true(new_key ~= old_key)

    local old_gapless = {
      schema = "consensus.consensus_reached.v1",
      proposal_id = target.proposal_id,
      decision = "reject",
      body = "Old gapless reject must not replay.",
      angle_results = {},
      dedup_key = "consensus:" .. target.dedup_key,
      source_ref = target.source_ref,
    }
    local seeded = seed_cache(old_key, result_memo.encode(old_gapless), run_opts)
    t.eq(seeded.exit_code, 0)

    mock_gate_synthesis_reject("missing regression test")
    local first = run_namespaced_decide(target, run_opts)
    t.eq(first.exit_code, 0)
    t.eq(first.raises[1].queue, "consensus_reached")
    t.eq(first.raises[1].payload.decision, "reject")
    t.eq(first.raises[1].payload.blocking_gap, "missing regression test")
    t.is_nil(first.raises[1].payload.body:find("Old gapless reject", 1, true))

    local second = run_namespaced_decide(target, run_opts)
    t.eq(second.exit_code, 0)
    t.eq(result_memo.encode(second.raises[1].payload), result_memo.encode(first.raises[1].payload))
    t.eq(second.raises[1].payload.blocking_gap, "missing regression test")
    t.eq(#codex_calls(), 7)
  end,

  test_concurrent_same_dedup_converge_loser_replays_winner_reached_payload = function()
    local target = proposal({
      dedup_key = "proposal-42-v1/concurrent",
      verdict_mode = "gate",
    })
    local memo_key = core.result_memo_key(target.dedup_key)
    local memo_cache = {}
    local emitted = {}
    local phase = "winner"
    local loser_memo_reads = 0
    local lock_entries = 0
    local memo_saves = 0
    local original_cache_get = cache_get
    local original_cache_set = cache_set

    without_live_codex_runs(function()
      with_global_overrides({
        cache_get = function(key)
          if key ~= memo_key then
            return original_cache_get(key)
          end
          if phase == "loser" then
            loser_memo_reads = loser_memo_reads + 1
            if loser_memo_reads == 1 then
              return nil
            end
          end
          return memo_cache[key]
        end,
        cache_set = function(key, value)
          if key ~= memo_key then
            return original_cache_set(key, value)
          end
          memo_saves = memo_saves + 1
          memo_cache[key] = value
        end,
        with_lock = function(key, fn)
          t.eq(key, memo_key)
          lock_entries = lock_entries + 1
          return fn()
        end,
        raise = function(queue, payload)
          table.insert(emitted, { queue = queue, payload = payload })
        end,
      }, function()
        mock_unanimous("approve", "Winner approves")
        decide_department.pipeline(namespaced_event(target))

        phase = "loser"
        mock_converge("Loser abstains")

        -- Deterministic replay of the concurrent double miss: both deliveries
        -- read nil, the winner computes approve and memoizes under the flock,
        -- then the loser enters the flock after computing converge. The test runs
        -- the winner first and returns the loser's earlier nil read on demand;
        -- its lock-scoped re-read must observe and emit the winner's payload.
        decide_department.pipeline(namespaced_event(target))
      end)
    end)

    t.eq(lock_entries, 2)
    t.eq(loser_memo_reads, 2)
    t.eq(memo_saves, 1)
    t.eq(#emitted, 2)
    t.eq(emitted[1].queue, "consensus_reached")
    t.eq(emitted[2].queue, "consensus_reached")
    t.eq(emitted[1].payload.decision, "approve")
    t.eq(result_memo.encode(emitted[2].payload), result_memo.encode(emitted[1].payload))
    t.is_nil(emitted[2].payload.body:find("Loser abstains", 1, true))

    local calls = codex_calls()
    t.eq(#calls, 10)
    for index = 4, 6 do
      t.is_true(calls[index].stdout:find(verdict_label .. " abstain", 1, true) ~= nil)
    end
    t.is_true(calls[10].stdout:find("converge: concurrent loser remains unresolved + inspect", 1, true) ~= nil)
  end,
}
