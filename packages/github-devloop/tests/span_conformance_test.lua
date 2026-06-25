local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t
local span = require("core.span_conformance")

local function contains_error(errors, needle)
  for _, err in ipairs(errors or {}) do
    local text = tostring(err.message or err)
    if text:find(needle, 1, true) ~= nil then
      return true
    end
  end
  return false
end

local function transition_source(contract_body)
  return [[
return function(M, h)
  local responsibility_signature = h.responsibility_signature; local span_contract = h.span_contract
  return {
    from_state = "implementing",
    responsibility_signature = responsibility_signature({
      state_kind = "worker",
    }),
]] .. contract_body .. [[
  }
end
]]
end

return {
  test_current_tree_has_no_gspan_errors = function()
    t.eq(#core.span_conformance_errors(), 0)
  end,

  test_completion_comment_key_start_wording_fails = function()
    local errors = span.errors_from_sources({
      ["packages/github-devloop/core/strings.lua"] = [[
local strings = { en = { implementation_started = "github-devloop implementation started" } }
]],
      ["packages/github-devloop/core/requests/lifecycle.lua"] = [[
function M.build_implementing_comment_request(repo, issue_number, ready, worktree, branch, head_sha)
  return { body = M.comment_string("implementation_started") .. "\nHead: " .. tostring(head_sha) }
end
]],
    })
    t.is_true(contains_error(errors, "completion/output comment uses start wording key"), "missing wording key error")
    t.is_true(contains_error(errors, "implementation_started"), "missing key in message")
  end,

  test_completion_comment_literal_start_wording_fails = function()
    local errors = span.errors_from_sources({
      ["packages/github-devloop/core/requests/lifecycle.lua"] = [[
function M.build_implementing_comment_request(repo, issue_number, ready, worktree, branch, head_sha)
  return { body = "github-devloop implementation started" .. "\nHead: " .. tostring(head_sha) }
end
]],
    })
    t.is_true(contains_error(errors, "completion/output comment uses start wording literal"), "missing wording literal error")
  end,

  test_spawn_before_declared_start_predecessor_fails = function()
    local errors = span.errors_from_sources({
      ["libraries/devloop/restart/issue/transitions/implementing.lua"] = transition_source([[
    span_contract = span_contract({
      department = "implement",
      durable_start_marker = "implement-attempt:v1",
      spawn_predecessor = "raise_implementing_state",
    }),
]]),
      ["packages/github-devloop/departments/implement/main.lua"] = [[
local function raise_implementing_state(repo, issue_number, ready)
  local marker = core.implement_attempt_marker(ready.proposal_id, ready.dedup_key, 1, now())
  raise("github-proxy.github_issue_comment_request", { body = marker })
end

local result = spawn_codex_sync({ prompt = prompt })
raise_implementing_state(repo, issue_number, ready)
]],
    })
    t.is_true(contains_error(errors, "spawn_codex_sync must be preceded by span start predecessor"), "missing spawn order error")
  end,

  test_declared_start_predecessor_can_bind_marker_through_shared_helper = function()
    local errors = span.errors_from_sources({
      ["libraries/devloop/restart/issue/transitions/implementing.lua"] = transition_source([[
    span_contract = span_contract({
      department = "implement",
      durable_start_marker = "implement-attempt:v1",
      spawn_predecessor = "raise_implementing_state",
    }),
]]),
      ["packages/github-devloop/departments/implement/main.lua"] = [[
local function raise_implementing_state(repo, issue_number, ready)
  local request = core.build_implementing_state_comment_request(repo, issue_number, ready)
  raise("github-proxy.github_issue_comment_request", request)
end

raise_implementing_state(repo, issue_number, ready)
local result = spawn_codex_sync({ prompt = prompt })
]],
      ["libraries/devloop/requests/lifecycle.lua"] = [[
function M.build_implementing_state_comment_request(repo, issue_number, ready)
  local marker = M.implement_attempt_marker(ready.proposal_id, ready.dedup_key, 1, now())
  return { body = marker }
end
]],
    })
    t.eq(#errors, 0)
  end,

  test_state_start_predecessor_can_bind_current_state_check = function()
    local errors = span.errors_from_sources({
      ["packages/github-devloop-pr/core/restart/transitions/fixing.lua"] = [[
return function(M, h)
  local responsibility_signature = h.responsibility_signature; local span_contract = h.span_contract
  return {
    from_state = "fixing",
    responsibility_signature = responsibility_signature({
      state_kind = "worker",
    }),
    span_contract = span_contract({
      department = "fix",
      durable_start_marker = "state:v1 fixing",
      spawn_predecessor = "precheck_fix_write_gate",
      spawn_function = "run_fix_attempt",
    }),
  }
end
]],
      ["packages/github-devloop-pr/departments/fix/main.lua"] = [[
local function validate_fix_write_gate_snapshot(pr, fix)
  local rechecked_state = core.current_entity_state(pr.comments, fix.proposal_id)
  if rechecked_state.state ~= "fixing" then
    return nil
  end
  return pr
end

local function precheck_fix_write_gate(repo, fix, branch)
  return validate_fix_write_gate_snapshot(pr, fix) ~= nil
end

local function run_fix_attempt(plan)
  local result = spawn_codex_sync({ prompt = prompt })
  return result
end

precheck_fix_write_gate(repo, fix, branch)
local outcome = run_fix_attempt(attempt_plan)
]],
    })
    t.eq(#errors, 0)
  end,

  test_long_running_dispatch_spawn_without_live_run_dedup_fails = function()
    local errors = span.errors_from_sources({
      ["packages/github-devloop-pr/core/restart/transitions/fixing.lua"] = [[
return function(M, h)
  local responsibility_signature = h.responsibility_signature; local span_contract = h.span_contract
  return {
    from_state = "fixing",
    responsibility_signature = responsibility_signature({
      state_kind = "worker",
    }),
    liveness_contract = {
      real_execution = {
        primitive = "fkst.codex_runs",
        match = {
          role = "fix",
          proposal_id = "state.proposal_id",
          dedup_key = "state.version",
        },
      },
    },
    span_contract = span_contract({
      department = "fix",
      durable_start_marker = "state:v1 fixing",
      spawn_predecessor = "precheck_fix_write_gate",
      spawn_function = "run_fix_attempt",
    }),
  }
end
]],
      ["packages/github-devloop-pr/departments/fix/main.lua"] = [[
local function validate_fix_write_gate_snapshot(pr, fix)
  local rechecked_state = core.current_entity_state(pr.comments, fix.proposal_id)
  if rechecked_state.state ~= "fixing" then
    return nil
  end
  return pr
end

local function precheck_fix_write_gate(repo, fix, branch)
  return validate_fix_write_gate_snapshot(pr, fix) ~= nil
end

local function run_fix_attempt(plan)
  return spawn_codex_sync({ prompt = prompt })
end

precheck_fix_write_gate(repo, fix, branch)
local outcome = run_fix_attempt(attempt_plan)
]],
    })
    t.is_true(contains_error(errors, "run_fix_attempt call must be preceded by dispatch_live_run_dedup"), "missing live-run dispatch dedup error")
  end,

  test_long_running_dispatch_spawn_with_live_run_dedup_passes = function()
    local errors = span.errors_from_sources({
      ["packages/github-devloop-pr/core/restart/transitions/fixing.lua"] = [[
return function(M, h)
  local responsibility_signature = h.responsibility_signature; local span_contract = h.span_contract
  return {
    from_state = "fixing",
    responsibility_signature = responsibility_signature({
      state_kind = "worker",
    }),
    liveness_contract = {
      real_execution = {
        primitive = "fkst.codex_runs",
        match = {
          role = "fix",
          proposal_id = "state.proposal_id",
          dedup_key = "state.version",
        },
      },
    },
    span_contract = span_contract({
      department = "fix",
      durable_start_marker = "state:v1 fixing",
      spawn_predecessor = "precheck_fix_write_gate",
      spawn_function = "run_fix_attempt",
    }),
  }
end
]],
      ["packages/github-devloop-pr/departments/fix/main.lua"] = [[
local function validate_fix_write_gate_snapshot(pr, fix)
  local rechecked_state = core.current_entity_state(pr.comments, fix.proposal_id)
  if rechecked_state.state ~= "fixing" then
    return nil
  end
  return pr
end

local function precheck_fix_write_gate(repo, fix, branch)
  return validate_fix_write_gate_snapshot(pr, fix) ~= nil
end

local function run_fix_attempt(plan)
  return spawn_codex_sync({ prompt = prompt })
end

precheck_fix_write_gate(repo, fix, branch)
if core.dispatch_live_run_dedup("fix", attempt_plan.fix.proposal_id, attempt_plan.fix.version) then
  return
end
local outcome = run_fix_attempt(attempt_plan)
]],
    })
    t.eq(#errors, 0)
  end,

  test_worker_span_contract_declaration_reuses_strict_contract = function()
    local rows = {}
    for index, row in ipairs(core.restart_transition_table()) do
      rows[index] = row
    end
    for _, row in ipairs(rows) do
      if row.from_state == "implementing" then
        row.span_contract = nil
      end
    end
    local errors = core.strict_restart_responsibility_contract_errors(rows)
    t.is_true(contains_error(errors, "implementing: worker row must declare span_contract"), "missing strict span error")
  end,

  test_source_list_tracks_old_gspan_scan_surface = function()
    local listed = {}
    for _, path in ipairs(span.source_paths()) do
      listed[path] = true
    end
    t.eq(listed["libraries/devloop/requests/lifecycle.lua"], true)
    t.eq(listed["libraries/devloop/restart/issue/transitions/implementing.lua"], true)
    t.eq(listed["packages/github-devloop/departments/implement/main.lua"], true)
    t.eq(listed["packages/github-devloop-pr/core/restart/transitions/fixing.lua"], true)
    t.eq(listed["packages/github-devloop-pr/departments/fix/main.lua"], true)
  end,
}
