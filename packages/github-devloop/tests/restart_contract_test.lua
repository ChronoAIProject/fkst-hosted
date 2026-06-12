local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local function has_value(values, expected)
  for _, value in ipairs(values or {}) do
    if value == expected then
      return true
    end
  end
  return false
end

local function copy_rows(rows)
  local copied = {}
  for index, row in ipairs(rows or {}) do
    local next_row = {}
    for key, value in pairs(row) do
      if type(value) == "table" then
        local nested = {}
        for nested_key, nested_value in pairs(value) do
          nested[nested_key] = nested_value
        end
        next_row[key] = nested
      else
        next_row[key] = value
      end
    end
    copied[index] = next_row
  end
  return copied
end

local function parse_marker_builders(paths)
  local families = {}
  for _, path in ipairs(paths) do
    local text = file.read(path)
    for family in text:gmatch("fkst:github%-devloop:([%w%-]+):v1") do
      families[family] = families[family] or {}
    end
    for family, attrs in pairs(families) do
      local family_pattern = "fkst:github%-devloop:" .. family:gsub("%-", "%%-") .. ":v1"
      local start_pos = text:find(family_pattern)
      if start_pos ~= nil then
        local function_pos = text:sub(1, start_pos):match("^.*()\nfunction M%.[^\n]+")
        local next_function = text:find("\nfunction M%.", start_pos + 1)
        local block = text:sub(function_pos or start_pos, next_function or #text)
        for attr in block:gmatch('" ([%w_]+)="') do
          attrs[attr] = true
        end
        for attr in block:gmatch('([%w_]+)="') do
          attrs[attr] = true
        end
      end
    end
  end
  return families
end

local function marker_builder_paths()
  return {
    "packages/github-devloop/core/state.lua",
    "packages/github-devloop/core/markers.lua",
    "packages/github-devloop/core/convergence.lua",
    "packages/github-devloop/core/dependencies.lua",
    "packages/github-devloop/core/decompose.lua",
    "packages/github-devloop/core/work_card.lua",
  }
end

local function table_by_state()
  local by_state = {}
  for _, row in ipairs(core.restart_transition_table()) do
    by_state[row.from_state] = row
  end
  return by_state
end

local function rows_by_state(rows)
  local by_state = {}
  for _, row in ipairs(rows or {}) do
    by_state[row.from_state] = row
  end
  return by_state
end

local function allowed_extra_transition(state, next_state)
  return state == "reviewing" and next_state == "blocked"
end

return {
  test_persistence_class_is_declared = function()
    t.eq(core.persistence_class(), "saga")
  end,

  test_executable_restart_table_covers_non_terminal_states = function()
    local expected = {
      "thinking",
      "ready",
      "implementing",
      "pr-open",
      "reviewing",
      "merge-ready",
      "merging",
      "fixing",
      "review-meta",
      "blocked",
    }
    local by_state = table_by_state()
    for _, state in ipairs(expected) do
      local row = by_state[state]
      t.is_true(row ~= nil)
      t.eq(row.from_state, state)
      t.is_true(type(row.to_states) == "table")
      t.is_true(type(row.driving_queue) == "string" and row.driving_queue ~= "")
      t.is_true(type(row.payload_builder) == "function")
      t.is_true(type(row.dedup_shape) == "string" and row.dedup_shape ~= "")
      t.is_true(type(row.required_facts) == "table" and #row.required_facts > 0)
      t.is_true(type(row.payload_fields) == "table")
      t.is_true(type(row.version_identity) == "string" and row.version_identity ~= "")
      t.is_true(type(row.effects) == "table")
      t.is_true(tonumber(row.effects.intent_count) ~= nil)
      t.is_true(type(row.effects.kinds) == "table")
      t.eq(#row.effects.kinds, row.effects.intent_count)
      t.is_true(type(row.effects.completeness) == "string" and row.effects.completeness ~= "")
    end
    t.eq(#core.restart_transition_table(), #expected)
  end,

  test_restart_table_matches_state_graph_and_stage_rank = function()
    local by_state = table_by_state()
    local expected = {
      thinking = true,
      ready = true,
      implementing = true,
      ["pr-open"] = true,
      reviewing = true,
      ["merge-ready"] = true,
      merging = true,
      fixing = true,
      ["review-meta"] = true,
      blocked = true,
    }
    for state, next_states in pairs(core._state_graph) do
      if expected[state] then
        local row = by_state[state]
        t.is_true(row ~= nil)
        for _, next_state in ipairs(row.to_states) do
          t.is_true(has_value(next_states, next_state) or allowed_extra_transition(state, next_state))
        end
        t.is_true(core.stage_rank(state) > 0)
      end
    end
    for state in pairs(expected) do
      t.is_true(by_state[state] ~= nil)
    end
  end,

  test_restart_required_facts_declare_freshness_modes = function()
    for _, row in ipairs(core.restart_transition_table()) do
      local saw_marker = false
      for _, required in ipairs(row.required_facts) do
        t.is_true(type(required.family) == "string" and required.family ~= "")
        t.is_true(required.freshness == "marker-read" or required.freshness == "fetch-before-compare")
        if required.freshness == "marker-read" then
          saw_marker = true
        end
      end
      t.is_true(saw_marker)
    end
  end,

  test_restart_payload_fields_are_covered_by_durable_fields = function()
    local errors = core.restart_field_coverage_errors()
    t.eq(#errors, 0)
  end,

  test_multi_effect_rows_declare_and_call_completeness_derivation = function()
    local by_state = table_by_state()
    t.eq(by_state.ready.effects.intent_count, 3)
    t.eq(by_state.ready.effects.kinds[1], "result-marker")
    t.eq(by_state.ready.effects.kinds[2], "ready-label")
    t.eq(by_state.ready.effects.kinds[3], "devloop_ready")
    t.eq(by_state.ready.effects.completeness_derivation, "result_effects_complete")
    t.eq(by_state.blocked.effects.intent_count, 2)
    t.eq(by_state.blocked.effects.completeness_derivation, "decompose_children_complete")
    t.eq(#core.restart_effect_contract_errors(), 0)
  end,

  test_multi_effect_contract_rejects_marker_only_rows = function()
    local rows = copy_rows(core.restart_transition_table())
    local ready = rows_by_state(rows).ready
    ready.effects.completeness_derivation = nil
    local errors = core.restart_effect_contract_errors(rows)
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("ready", 1, true) ~= nil)
    t.is_true(errors[1]:find("completeness derivation", 1, true) ~= nil)
  end,

  test_restart_field_coverage_catches_374_shape_missing_gate_baseline = function()
    local rows = copy_rows(core.restart_transition_table())
    rows_by_state(rows).fixing.payload_fields.gate_baseline_sha = nil
    local errors = core.restart_field_coverage_errors(rows)
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("fixing.gate_baseline_sha", 1, true) ~= nil)
    t.is_true(errors[1]:find("missing required replay payload field", 1, true) ~= nil)
  end,

  test_declared_marker_fields_exist_in_marker_builders = function()
    local parsed = parse_marker_builders(marker_builder_paths())
    for family, attrs in pairs(core.restart_durable_marker_fields()) do
      t.is_true(parsed[family] ~= nil, "missing marker family " .. tostring(family))
      for attr in pairs(attrs) do
        t.is_true(parsed[family][attr] == true, "missing marker attr " .. tostring(family) .. "." .. tostring(attr))
      end
    end
  end,

  test_source_ref_derivations_are_declared = function()
    local derivations = core.restart_source_ref_derivations()
    t.eq(derivations.issue, true)
    t.eq(derivations.pr, true)
    t.eq(derivations.entity, true)
  end,

  test_replay_payload_fields_resolve_from_declared_table_map = function()
    local state = {
      state = "fixing",
      version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-12T00-00-00Z",
    }
    local fields = core.resolve_replay_payload_fields(table_by_state().fixing, state, {
      issue = {
        repo = "owner/repo",
        source_ref = core.issue_source_ref("owner/repo", 42),
      },
      proposal_id = "github-devloop/issue/owner/repo/42",
      link = {
        pr_number = 7,
      },
      feedback = {
        review_proposal_id = "github-devloop/pr-review/owner/repo/7/v/def456",
        review_dedup_key = "consensus:github-devloop/pr-review/owner/repo/7/v/def456/review",
        reviewed_head_sha = "def456",
        gate_baseline_sha = "abc123",
        blocking_gap = "missing guard",
      },
    })
    t.eq(fields.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(fields.pr_number, 7)
    t.eq(fields.version, state.version)
    t.eq(fields.review_proposal_id, "github-devloop/pr-review/owner/repo/7/v/def456")
    t.eq(fields.review_dedup_key, "consensus:github-devloop/pr-review/owner/repo/7/v/def456/review")
    t.eq(fields.reviewed_head_sha, "def456")
    t.eq(fields.gate_baseline_sha, "abc123")
    t.eq(fields.blocking_gap, "missing guard")
    t.eq(fields.source_ref.ref, "owner/repo#pr/7")
  end,

  test_replayer_gathers_fetch_before_compare_pr_facts_from_table = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-12T00-00-00Z"
    local issue = {
      repo = "owner/repo",
      number = 42,
      source_ref = core.issue_source_ref("owner/repo", 42),
    }
    local state = {
      state = "pr-open",
      version = version,
    }
    local issue_comments = {
      { body = core.state_marker(proposal_id, "pr-open", version), author_login = "fkst-test-bot" },
      { body = core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", version, "dev"), author_login = "fkst-test-bot" },
    }
    t.mock_command(core.gh_pr_view_observe_cmd("owner/repo", 7), {
      stdout = '{"headRefName":"devloop-owner-repo-42-01HY","headRefOid":"def456","baseRefName":"dev","state":"OPEN","updatedAt":"2026-06-03T02:03:04Z","comments":[]}\n',
      stderr = "",
      exit_code = 0,
    })
    local gathered = core.gather_replay_required_facts(table_by_state()["pr-open"], issue, state, {
      proposal_id = proposal_id,
      current = { comments = issue_comments },
      snapshot = {
        comments = issue_comments,
        prs = {
          {
            number = 7,
            current = {
              head_sha = "stale",
              head_ref_name = "stale",
              base_ref_name = "dev",
              state = "OPEN",
              comments = {},
            },
          },
        },
      },
    })
    t.eq(gathered.snapshot.prs[1].current.head_sha, "def456")
    t.eq(#t.command_calls(), 1)
  end,

  test_replayer_fetch_before_compare_ignores_caller_fresh_flag = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-12T00-00-00Z"
    local issue = {
      repo = "owner/repo",
      number = 42,
      source_ref = core.issue_source_ref("owner/repo", 42),
    }
    local state = {
      state = "pr-open",
      version = version,
    }
    local issue_comments = {
      { body = core.state_marker(proposal_id, "pr-open", version), author_login = "fkst-test-bot" },
      { body = core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", version, "dev"), author_login = "fkst-test-bot" },
    }
    t.mock_command(core.gh_pr_view_observe_cmd("owner/repo", 7), {
      stdout = '{"headRefName":"devloop-owner-repo-42-01HY","headRefOid":"def456","baseRefName":"dev","state":"OPEN","updatedAt":"2026-06-03T02:03:04Z","comments":[]}\n',
      stderr = "",
      exit_code = 0,
    })
    local gathered = core.gather_replay_required_facts(table_by_state()["pr-open"], issue, state, {
      proposal_id = proposal_id,
      current = { comments = issue_comments },
      snapshot = {
        fresh = true,
        fetch_before_compare = {
          ["pr-head"] = true,
        },
        comments = issue_comments,
        prs = {
          {
            number = 7,
            current = {
              head_sha = "stale",
              head_ref_name = "stale",
              base_ref_name = "dev",
              state = "OPEN",
              comments = {},
            },
          },
        },
      },
    })
    t.eq(gathered.snapshot.prs[1].current.head_sha, "def456")
    t.eq(#t.command_calls(), 1)
  end,

  test_observe_issue_replay_is_table_driven = function()
    local text = file.read("packages/github-devloop/departments/observe_issue/main.lua")
    t.is_true(text:find("core.replay_from_table", 1, true) ~= nil)
    t.eq(text:find("build_replayed_fixing_payload", 1, true), nil)
    t.eq(text:find("build_devloop_review_meta_payload", 1, true), nil)
    t.eq(text:find("build_decompose_replay_payload", 1, true), nil)
    t.eq(text:find("build_devloop_reviewing_payload", 1, true), nil)
  end,
}
