local t = fkst.test

local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"
local stance_label = "⟦FKST:STANCE⟧"

local function nonce()
  return tostring({}):gsub("[^%w._-]", "_")
end

local function opts(name)
  return {
    env = {
      FKST_RUNTIME_ROOT = "/tmp/fkst-packages-test/consensus-restart/" .. tostring(now()) .. "/" .. nonce() .. "/" .. name,
    },
  }
end

local function proposal(extra)
  local value = {
    schema = "consensus.proposal.v1",
    proposal_id = "proposal-restart-42",
    title = "Judge a durable proposal after restart",
    body = "Judge the supplied change against the repository.",
    content_fetch = "fetch-source --ref demo/consensus/42 --full",
    angles = { "teleology", "parsimony", "fidelity" },
    dedup_key = "proposal-restart-42-v1",
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

local function run_decide(value, run_opts)
  return t.run_department("departments/decide/main.lua", {
    queue = "proposal",
    payload = value,
  }, run_opts)
end

local function synthesis_stdout(line)
  local text = tostring(line or "")
  if text:find("converge:", 1, true) == 1 then
    text = text .. "\nopen: unresolved synthesis disagreement"
  end
  return text .. "\n"
end

local function shell_single_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function checkout_root_exists_cmd(path)
  local quoted = shell_single_quote(path)
  return "test -d " .. quoted .. " && test -e " .. quoted .. "/.git"
end

local function mock_judgment_runtime()
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/consensus-restart/runtime",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_checkout(path, exists)
  t.mock_command(checkout_root_exists_cmd(path), {
    stdout = "",
    stderr = "",
    exit_code = exists and 0 or 1,
  })
end

local function mock_checkout_sequence(path, sequence)
  for _, exists in ipairs(sequence) do
    mock_checkout(path, exists)
  end
end

local function mock_full_debate()
  for _, stdout in ipairs({
    verdict_label .. " approve\n" .. reply_label .. " Teleology angle accepts a small adapter.\n",
    verdict_label .. " abstain\n" .. reply_label .. " Parsimony wants the retry boundary explicit.\n",
    verdict_label .. " approve\n" .. reply_label .. " Fidelity accepts removing duplicate wiring.\n",
    stance_label .. " defend\n" .. verdict_label .. " approve\n" .. reply_label .. " Teleology still approves.\n",
    stance_label .. " defend\n" .. verdict_label .. " abstain\n" .. reply_label .. " Parsimony still abstains.\n",
    stance_label .. " defend\n" .. verdict_label .. " approve\n" .. reply_label .. " Fidelity still approves.\n",
    synthesis_stdout("malformed synthesis output"),
    synthesis_stdout("converge: retry ownership remains unresolved + inspect the retry owner record"),
  }) do
    t.mock_command("codex exec", {
      stdout = stdout,
      stderr = "",
      exit_code = 0,
    })
  end
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

local function assert_phase_sequence(calls)
  for index = 1, 3 do
    t.is_true(calls[index].stdin:find("Judge this proposal from one whole-picture philosopher seat.", 1, true) ~= nil)
  end
  for index = 4, 6 do
    t.is_true(calls[index].stdin:find("Phase R rebuttal", 1, true) ~= nil)
  end
  t.is_true(calls[7].stdin:find("This is the first synthesis attempt.", 1, true) ~= nil)
  t.is_true(calls[8].stdin:find("Repair attempt:", 1, true) ~= nil)
end

local function assert_checkout_call(call, expected_worktree, rejected_worktree)
  t.is_true(call.rendered:find(" -C " .. expected_worktree .. " ", 1, true) ~= nil)
  t.is_nil(call.rendered:find("/judgment-worktrees/", 1, true))
  if rejected_worktree ~= nil then
    t.is_nil(call.rendered:find(rejected_worktree, 1, true))
  end
  t.is_true(call.stdin:find("read-only checkout of the judged repository", 1, true) ~= nil)
  t.is_nil(call.stdin:find("empty runtime scratch directory", 1, true))
end

local function assert_checkout_calls(calls, expected_worktree, rejected_worktree)
  t.eq(#calls, 8)
  for _, call in ipairs(calls) do
    assert_checkout_call(call, expected_worktree, rejected_worktree)
  end
  assert_phase_sequence(calls)
end

local function command_count(rendered)
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    if call.rendered == rendered then
      count = count + 1
    end
  end
  return count
end

local function assert_validation_counts(candidate, candidate_count, fallback_count)
  t.eq(command_count(checkout_root_exists_cmd(candidate)), candidate_count)
  t.eq(command_count(checkout_root_exists_cmd(".")), fallback_count)
end

local function assert_no_judgment_dir_created()
  for _, call in ipairs(t.command_calls()) do
    t.is_nil(call.rendered:find("mkdir -p", 1, true))
  end
end

return {
  test_proposal_worktree_runs_every_seat_without_scratch_mkdir = function()
    local worktree = "/tmp/live-consensus-checkout"
    mock_judgment_runtime()
    mock_checkout_sequence(worktree, { true, true, true, true, true, true, true, true })
    mock_full_debate()

    local result = run_decide(proposal({ worktree = worktree }), opts("live-checkout"))

    t.eq(result.exit_code, 0)
    assert_checkout_calls(codex_calls(), worktree)
    assert_validation_counts(worktree, 8, 0)
    assert_no_judgment_dir_created()
  end,

  test_stale_durable_worktree_falls_back_for_every_seat_after_restart = function()
    local stale_worktree = "/tmp/old-runtime-root/worktrees/review-42"
    mock_judgment_runtime()
    mock_checkout_sequence(stale_worktree, { false, false, false, false, false, false, false, false })
    mock_checkout_sequence(".", { true, true, true, true, true, true, true, true })
    mock_full_debate()

    local result = run_decide(proposal({
      dedup_key = "proposal-restart-42-v1/stale-checkout",
      worktree = stale_worktree,
    }), opts("stale-checkout"))

    t.eq(result.exit_code, 0)
    assert_checkout_calls(codex_calls(), ".", stale_worktree)
    assert_validation_counts(stale_worktree, 8, 8)
    assert_no_judgment_dir_created()
  end,

  test_checkout_disappearing_after_blind_revalidates_every_later_seat = function()
    local worktree = "/tmp/phased-consensus-checkout"
    mock_judgment_runtime()
    mock_checkout_sequence(worktree, { true, true, true, false, false, false, false, false })
    mock_checkout_sequence(".", { true, true, true, true, true })
    mock_full_debate()

    local result = run_decide(proposal({
      dedup_key = "proposal-restart-42-v1/phased-checkout",
      worktree = worktree,
    }), opts("phased-checkout"))

    t.eq(result.exit_code, 0)
    local calls = codex_calls()
    t.eq(#calls, 8)
    for index = 1, 3 do
      assert_checkout_call(calls[index], worktree)
    end
    for index = 4, 8 do
      assert_checkout_call(calls[index], ".", worktree)
    end
    assert_phase_sequence(calls)
    assert_validation_counts(worktree, 8, 5)
    assert_no_judgment_dir_created()
  end,

  test_stale_durable_worktree_fails_closed_when_fallback_is_not_a_checkout = function()
    local stale_worktree = "/tmp/old-runtime-root/worktrees/review-42"
    mock_judgment_runtime()
    mock_checkout(stale_worktree, false)
    mock_checkout(".", false)

    local result = run_decide(proposal({
      dedup_key = "proposal-restart-42-v1/missing-fallback",
      worktree = stale_worktree,
    }), opts("missing-fallback"))

    t.is_true(result.exit_code ~= 0)
    t.is_true(tostring(result.error):find("judgment-worktree-unavailable", 1, true) ~= nil)
    t.eq(#codex_calls(), 0)
    assert_validation_counts(stale_worktree, 1, 1)
    assert_no_judgment_dir_created()
  end,
}
