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
    "packages/github-devloop/core/impl_failure.lua",
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
  return (state == "reviewing" and next_state == "blocked")
    or (state == "impl-failed" and next_state == "implementing")
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
      "impl-failed",
      "pr-open",
      "reviewing",
      "merge-ready",
      "merging",
      "fixing",
      "review-meta",
      "blocked",
      "merged",
    }
    local by_state = table_by_state()
    for _, state in ipairs(expected) do
      local row = by_state[state]
      t.is_true(row ~= nil)
      t.eq(row.from_state, state)
      t.is_true(type(row.to_states) == "table")
      t.is_true(type(row.terminal) == "boolean")
      if row.terminal == false then
        t.is_true(type(row.driving_queue) == "string" and row.driving_queue ~= "")
        t.is_true(type(row.output_obligation) == "table")
        t.is_true(type(row.budget) == "table")
        t.is_true(type(row.on_timeout) == "table")
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
    end
    t.eq(#core.restart_transition_table(), #expected)
  end,

  test_liveness_contract_declares_terminal_taxonomy_and_backstop = function()
    local errors = core.liveness_contract_errors()
    t.eq(#errors, 0)
    local terminals = core.liveness_terminal_states()
    t.eq(#terminals, 1)
    t.eq(terminals[1], "merged")
    local by_state = table_by_state()
    t.eq(by_state["impl-failed"].terminal, false)
    t.eq(by_state["impl-failed"].on_timeout.queue, "devloop_ready")
    t.eq(by_state["impl-failed"].reentry_commands[1], "reready")
    for _, row in ipairs(core.restart_transition_table()) do
      if row.terminal == false then
        t.is_true(row.output_obligation ~= nil)
        t.is_true(tonumber(row.budget.minutes) > 0)
        t.eq(row.on_timeout.action, "redrive")
        t.eq(row.on_timeout.queue, row.driving_queue)
        t.is_true(row.on_timeout.queue ~= "none")
      end
    end
  end,

  test_reentry_commands_are_supported_by_operator_parser = function()
    for _, row in ipairs(core.restart_transition_table()) do
      for _, command_name in ipairs(row.reentry_commands or {}) do
        local fact = core.operator_command_fact({
          {
            id = "IC_" .. tostring(row.from_state) .. "_" .. tostring(command_name),
            body = "fkst: " .. tostring(command_name),
            author_login = "fkst-test-bot",
            created_at = "2026-06-04T03:00:00Z",
          },
        }, command_name)
        t.is_true(fact ~= nil, "unsupported reentry command " .. tostring(command_name))
      end
    end
  end,

  test_liveness_contract_rejects_non_terminal_without_output_obligation = function()
    local rows = copy_rows(core.restart_transition_table())
    rows_by_state(rows).ready.output_obligation = nil
    local errors = core.liveness_contract_errors(rows)
    t.eq(#errors, 1)
    t.is_true(errors[1]:find("ready", 1, true) ~= nil)
    t.is_true(errors[1]:find("output_obligation", 1, true) ~= nil)
  end,

  test_liveness_timeout_versions_preserve_lineage_and_attempts = function()
    local row = table_by_state()["impl-failed"]
    local base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local state = {
      state = "impl-failed",
      version = base,
      marker_created_at = "2026-06-03T01:02:03Z",
    }
    local decision = core.liveness_timeout_decision(row, state, core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"))
    t.eq(decision.action, "redrive")
    t.eq(decision.attempt, 1)
    t.eq(core.version_timeout_round(decision.version, "impl-failed"), 1)
    t.eq(core.strip_transition_version_suffixes(decision.version), core.strip_transition_version_suffixes(base))
    local over = {
      state = "impl-failed",
      version = base .. "/timeout/impl-failed/3",
      marker_created_at = "2026-06-03T01:02:03Z",
    }
    local escalated = core.liveness_timeout_decision(row, over, core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"))
    t.eq(escalated.action, "escalate")
  end,

  test_liveness_timeout_escalates_thinking_to_reconcile_event = function()
    local row = table_by_state().thinking
    local base = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local raised = {}
    local original_log_raise = core.log_raise
    core.log_raise = function(_, _, queue, payload)
      table.insert(raised, { queue = queue, payload = payload })
    end
    local ok, err = pcall(function()
      local applied = core.maybe_timeout_redrive_from_table("observe_issue", {
        repo = "owner/repo",
        number = 42,
        source_ref = core.issue_source_ref("owner/repo", 42),
      }, {
        state = "thinking",
        version = base .. "/timeout/thinking/3",
        proposal_id = "github-devloop/issue/owner/repo/42",
        marker_created_at = "2026-06-03T01:02:03Z",
      }, row, {
        proposal_id = "github-devloop/issue/owner/repo/42",
        now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"),
      })
      t.eq(applied, true)
    end)
    core.log_raise = original_log_raise
    if not ok then
      error(err)
    end
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "devloop_reconcile")
    t.eq(raised[1].payload.schema, "github-devloop.reconcile.v1")
    t.eq(raised[1].payload.round, 3)
    t.eq(raised[1].payload.base_version, base)
    t.eq(raised[1].payload.dedup_key, "reconcile:" .. base .. "/loop/3")
  end,

  test_liveness_timeout_escalates_reviewing_to_review_reconcile_event = function()
    local row = table_by_state().reviewing
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    local head_sha = "def456"
    local review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, version, head_sha)
    local raised = {}
    local original_log_raise = core.log_raise
    core.log_raise = function(_, _, queue, payload)
      table.insert(raised, { queue = queue, payload = payload })
    end
    local ok, err = pcall(function()
      local applied = core.maybe_timeout_redrive_from_table("observe_pr", {
        repo = "owner/repo",
        number = 42,
        source_ref = core.issue_source_ref("owner/repo", 42),
      }, {
        state = "reviewing",
        version = version .. "/timeout/reviewing/3",
        proposal_id = "github-devloop/issue/owner/repo/42",
        marker_created_at = "2026-06-03T01:02:03Z",
      }, row, {
        proposal_id = "github-devloop/issue/owner/repo/42",
        source_ref = core.pr_source_ref("owner/repo", 7),
        review_proposal_id = review_proposal_id,
        head_sha = head_sha,
        now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"),
      })
      t.eq(applied, true)
    end)
    core.log_raise = original_log_raise
    if not ok then
      error(err)
    end
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "devloop_review_reconcile")
    t.eq(raised[1].payload.schema, "github-devloop.review-reconcile.v1")
    t.eq(raised[1].payload.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(raised[1].payload.review_proposal_id, review_proposal_id)
    t.eq(raised[1].payload.issue_version, core.safe_version_segment(version .. "/timeout/reviewing/3"))
    t.eq(raised[1].payload.head_sha, head_sha)
    t.eq(raised[1].payload.round, 3)
  end,

  test_liveness_timeout_escalation_has_observable_event_for_every_non_terminal_row = function()
    local original_log_raise = core.log_raise
    local raised = {}
    core.log_raise = function(_, _, queue, payload)
      table.insert(raised, { queue = queue, payload = payload })
    end
    local ok, err = pcall(function()
      for _, row in ipairs(core.restart_transition_table()) do
        if row.terminal == false then
          local before = #raised
          local base = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
          local state = {
            state = row.from_state,
            version = base .. "/timeout/" .. row.from_state .. "/3",
            proposal_id = "github-devloop/issue/owner/repo/42",
            marker_created_at = "2026-06-03T01:02:03Z",
          }
          local applied = core.maybe_timeout_redrive_from_table("observe_issue", {
            repo = "owner/repo",
            number = 42,
            source_ref = core.issue_source_ref("owner/repo", 42),
          }, state, row, {
            proposal_id = state.proposal_id,
            source_ref = row.from_state == "reviewing" and core.pr_source_ref("owner/repo", 7) or core.issue_source_ref("owner/repo", 42),
            review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, state.version, "def456"),
            head_sha = "def456",
            now_seconds = core.iso_timestamp_epoch_seconds("2026-06-04T01:02:03Z"),
          })
          t.eq(applied, true)
          t.eq(#raised, before + 1)
          t.is_true(raised[#raised].queue == "devloop_reconcile"
            or raised[#raised].queue == "devloop_review_reconcile"
            or raised[#raised].queue == "devloop_timeout_reconcile")
          t.eq(raised[#raised].queue == row.driving_queue, false)
          t.eq(tostring(raised[#raised].payload.dedup_key or ""):find("/timeout/" .. row.from_state .. "/4", 1, true), nil)
        end
      end
    end)
    core.log_raise = original_log_raise
    if not ok then
      error(err)
    end
  end,

  test_restart_table_matches_state_graph_and_stage_rank = function()
    local by_state = table_by_state()
    local expected = {
      thinking = true,
      ready = true,
      implementing = true,
      ["impl-failed"] = true,
      ["pr-open"] = true,
      reviewing = true,
      ["merge-ready"] = true,
      merging = true,
      fixing = true,
      ["review-meta"] = true,
      ["impl-failed"] = true,
      blocked = true,
      merged = true,
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
      if row.terminal == true then
        goto continue
      end
      local saw_marker = false
      for _, required in ipairs(row.required_facts) do
        t.is_true(type(required.family) == "string" and required.family ~= "")
        t.is_true(required.freshness == "marker-read" or required.freshness == "fetch-before-compare")
        if required.freshness == "marker-read" then
          saw_marker = true
        end
      end
      t.is_true(saw_marker)
      ::continue::
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

  test_pr_side_rows_declare_pr_state_label_projection = function()
    local by_state = table_by_state()
    for _, state in ipairs({ "pr-open", "reviewing", "merge-ready", "merging", "fixing" }) do
      local row = by_state[state]
      t.is_true(row ~= nil)
      t.is_true(has_value(row.effects.kinds, "pr-state-label"), state .. " missing pr-state-label effect")
      t.is_true(
        tostring(row.effects.completeness or ""):find("PR-local state label projection", 1, true) ~= nil,
        state .. " missing PR-local label completeness text"
      )
    end
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
      },
    })
    t.eq(fields.proposal_id, "github-devloop/issue/owner/repo/42")
    t.eq(fields.pr_number, 7)
    t.eq(fields.version, state.version)
    t.eq(fields.review_proposal_id, "github-devloop/pr-review/owner/repo/7/v/def456")
    t.eq(fields.review_dedup_key, "consensus:github-devloop/pr-review/owner/repo/7/v/def456/review")
    t.eq(fields.reviewed_head_sha, "def456")
    t.eq(fields.gate_baseline_sha, "abc123")
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

  test_observe_pr_replay_is_table_driven = function()
    local text = file.read("packages/github-devloop/departments/observe_pr/main.lua")
    t.is_true(text:find("core.replay_from_table", 1, true) ~= nil)
    t.eq(text:find("build_replayed_fixing_payload", 1, true), nil)
    t.eq(text:find("build_decompose_replay_payload", 1, true), nil)
    t.eq(text:find("build_devloop_merge_ready_payload", 1, true), nil)
    t.eq(text:find("review_carry_over_marker", 1, true), nil)
  end,
}
