local devloop_base = require("devloop.base")
local h = require("tests.devloop_helpers")
local payloads_builders = require("devloop.payloads.builders")
local t = h.t
local core = h.core
local opts = h.opts
local find_raise = h.find_raise
local count_calls = h.count_calls
local entity_read_mocks = require("tests.entity_read_mock_helpers")
local conv_reconcile = require("devloop.convergence.reconcile")
local conv_rounds = require("devloop.convergence.rounds")
local convergence_shared = require("devloop.convergence.shared")
local m_builders = require("devloop.markers.builders")

local function encode_json_string(value)
  return h.encode_json_string(value)
end

local function encode_labels_json(labels)
  local rendered = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered, string.format('{"name":"%s"}', encode_json_string(label)))
  end
  return table.concat(rendered, ",")
end

local function comments_json(comments)
  local rendered = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered, h.render_comment(comment))
  end
  return table.concat(rendered, ",")
end

local function issue_list_json(issues)
  local rendered = {}
  for _, issue in ipairs(issues or {}) do
    table.insert(rendered, string.format(
      '{"number":%d,"title":"%s","body":"%s","createdAt":"%s","updatedAt":"%s","labels":[%s],"assignees":[%s],"author":{"login":"%s"}}',
      issue.number,
      encode_json_string(issue.title or "Issue"),
      encode_json_string(issue.body or ""),
      encode_json_string(issue.created_at or "2026-06-03T01:00:00Z"),
      encode_json_string(issue.updated_at or "2026-06-03T01:02:03Z"),
      encode_labels_json(issue.labels or {}),
      issue.assignees_json or '{"login":"fkst-test-bot"}',
      encode_json_string(issue.author_login or "fkst-test-bot")
    ))
  end
  return "[" .. table.concat(rendered, ",") .. "]"
end

local function mock_bot_env(value)
  h.mock_bot_env(value)
end

local function trusted_reintake_command(id)
  return {
    id = id or "IC_reintake_1",
    body = "fkst: reintake",
    author_login = devloop_base.trusted_bot_login(),
    created_at = "2026-06-04T03:00:00Z",
  }
end

local function trusted_comment(body, created_at)
  return {
    body = body,
    author_login = devloop_base.trusted_bot_login(),
    created_at = created_at,
  }
end

local function find_comment_body(raises, needle)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == "github-proxy.github_issue_comment_request"
      and raised.payload.body:find(needle, 1, true) ~= nil then
      return raised.payload
    end
  end
  return nil
end

local function find_label_add(raises, label)
  for _, raised in ipairs(raises or {}) do
    if raised.queue == "github-proxy.github_issue_label_request" then
      for _, value in ipairs((raised.payload or {}).add_labels or {}) do
        if tostring(value) == tostring(label) then
          return raised.payload
        end
      end
    end
  end
  return nil
end

local function default_intake_current()
  return {
    title = "Add retry backoff to failed widget sync",
    body = "Implement exponential backoff for widget sync retries. Acceptance: unit tests cover 1s, 2s, and capped retries.",
  }
end

local function expected_decision_key(payload, reintake_command, effective_updated_at)
  return devloop_base.intake_decision_dedup_key(payload.proposal_id, default_intake_current(), reintake_command, effective_updated_at)
end

local function assert_execution_request_chain(raises, payload, reintake_command)
  local expected_dedup = expected_decision_key(payload, reintake_command, payload.reintake_effect_updated_at)
  local request = find_raise(raises, "github-devloop.devloop_execute_request").payload
  t.eq(request.schema, "github-devloop.execution-request.v1")
  t.eq(request.proposal_id, payload.proposal_id)
  t.eq(request.dedup_key, expected_dedup)
  t.eq(request.service_class, "standard")
  t.eq(request.source_ref.ref, payload.source_ref.ref)
  t.eq(request.origin.package, "github-devloop-intake-default")
  t.eq(request.origin.route, "default")
  t.eq(request.origin.decision, "enable")
  t.eq(find_raise(raises, "consensus.proposal"), nil)
  t.eq(find_comment_body(raises, 'state="thinking"'), nil)
  t.eq(find_label_add(raises, "fkst-dev:thinking"), nil)
end

local function mock_intake_judge_view(labels, comments, extra)
  local fields = extra or {}
  local assignees_json = fields.assignees_json or '{"login":"fkst-test-bot"}'
  local assignee_stdout = string.format(
    '{"title":"%s","body":"%s","updatedAt":"%s","state":"%s","labels":[%s],"comments":[%s],"assignees":[%s],"author":{"login":"%s"}}\n',
    encode_json_string(fields.title or "Add retry backoff to failed widget sync"),
    encode_json_string(fields.body or "Implement exponential backoff for widget sync retries. Acceptance: unit tests cover 1s, 2s, and capped retries."),
    encode_json_string(fields.updated_at or "2026-06-03T01:02:03Z"),
    encode_json_string(fields.state or "OPEN"),
    encode_labels_json(labels or {}),
    comments_json(comments or {}),
    assignees_json,
    encode_json_string(fields.author_login or "fkst-test-bot"))
  entity_read_mocks.mock_issue_view_raw_selector(t, {}, "title,body,createdAt,updatedAt,labels,comments,state,assignees,author", {
    stdout = assignee_stdout,
  }, 2)
  entity_read_mocks.mock_issue_view_raw_selector(t, {}, "title,body,updatedAt,labels,comments,state", {
    stdout = string.format(
      '{"title":"%s","body":"%s","updatedAt":"%s","state":"%s","labels":[%s],"comments":[%s]}\n',
      encode_json_string(fields.title or "Add retry backoff to failed widget sync"),
      encode_json_string(fields.body or "Implement exponential backoff for widget sync retries. Acceptance: unit tests cover 1s, 2s, and capped retries."),
      encode_json_string(fields.updated_at or "2026-06-03T01:02:03Z"),
      encode_json_string(fields.state or "OPEN"),
      encode_labels_json(labels or {}),
      comments_json(comments or {})
    ),
  })
end

local function mock_intake_codex(stdout)
  local ok = { stdout = "", stderr = "", exit_code = 0 }
  for _ = 1, 4 do
    t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
      stdout = "fkst-test-bot",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command('printf %s "$FKST_DEVLOOP_MANAGED_BOT_LOGINS"', ok)
    t.mock_command('printf %s "$FKST_GITHUB_AUTHORIZED_LOGINS"', ok)
  end
  for _ = 1, 3 do
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop/runtime",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 2 do
    t.mock_command("test -d", { stdout = "", stderr = "", exit_code = 1 })
  end
  t.mock_command("install -d -m 0755", ok)
  t.mock_command("mktemp -d", {
    stdout = "/tmp/fkst-packages-test/github-devloop/runtime/context/.bundle-tmp.intake\n",
    stderr = "",
    exit_code = 0,
  })
  mock_intake_judge_view({}, {})
  entity_read_mocks.mock_issue_board_digest_list_raw(t, "owner/repo", {
    stdout = "[]\n",
  })
  entity_read_mocks.mock_issue_list_raw_command(t, core.gh_issue_list_recent_closed_cmd("owner/repo", 30), {
    stdout = issue_list_json({
      { number = 80, title = "Widget sync retry patch", labels = { "fingerprint:widget-sync" } },
      { number = 81, title = "Widget sync retry overflow fix", labels = { "fingerprint:widget-sync" } },
      { number = 82, title = "Widget sync timeout fix", labels = { "fingerprint:widget-sync" } },
    }) .. "\n",
  })
  t.mock_command("gh pr list", {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
  for _ = 1, 3 do
    t.mock_command(" > ", ok)
  end
  t.mock_command("python3 -c", ok)
  for _ = 1, 3 do
    t.mock_command("test -r", ok)
  end
  for _ = 1, 8 do
    t.mock_command("wc -c < ", {
      stdout = "1\n",
      stderr = "",
      exit_code = 0,
    })
  end
  t.mock_command("mkdir -p", ok)
  t.mock_command("codex exec", {
    stdout = stdout,
    stderr = "",
    exit_code = 0,
  })
end

local function candidate()
  return payloads_builders.build_devloop_intake_candidate_payload("owner/repo", 42, "2026-06-03T01:02:03Z")
end

local function reintake_candidate(command)
  local payload = candidate()
  payload.effect_id = expected_decision_key(payload, command, command.created_at)
  payload.dedup_key = core.intake_candidate_delivery_dedup_key(payload.proposal_id, payload.effect_id, payload.effect_id)
  payload.reintake_command_created_at = command.created_at
  payload.reintake_effect_updated_at = command.created_at
  return payload
end

local function run_judge(payload, run_opts)
  return t.run_department("departments/intake_judge/main.lua", {
    queue = "github-devloop-intake.devloop_intake_candidate",
    payload = payload,
  }, run_opts)
end

return {
  test_judge_reintake_rejudges_after_trusted_intake_marker = function()
    local command = trusted_reintake_command("IC_reintake_judge")
    local payload = reintake_candidate(command)
    mock_bot_env()
    mock_intake_judge_view({}, {
      m_builders.intake_decision_marker(payload.proposal_id, "escalate-to-class", expected_decision_key(payload, command, command.created_at), "standard"),
      command,
    })
    mock_intake_codex("⟦FKST:INTAKE⟧ enable\n⟦FKST:CLASS⟧ standard\n⟦FKST:REASON⟧ Class-level carrier; reintake enables after calibration.")

    local result = run_judge(payload, opts("intake-reintake"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 4)
    local command_comment = find_comment_body(result.raises, "operator command accepted: reintake")
    local intake_comment = find_comment_body(result.raises, 'decision="enable"')
    t.is_true(command_comment ~= nil)
    t.is_true(intake_comment ~= nil)
    t.is_true(command_comment.body:find('command="reintake"', 1, true) ~= nil)
    t.eq(find_label_add(result.raises, "fkst-dev:enabled").add_labels[1], "fkst-dev:enabled")
    assert_execution_request_chain(result.raises, payload, command)
    t.eq(count_calls("codex exec"), 1)
  end,

  test_judge_reintake_rejudges_terminal_blocked_issue_with_effect_timestamp_after_blocked = function()
    local command = trusted_reintake_command("IC_reintake_blocked_judge")
    command.created_at = "2026-06-04T03:00:00Z"
    local blocked_created_at = "2026-06-04T04:00:00Z"
    local payload = reintake_candidate(command)
    payload.effect_id = expected_decision_key(payload, command, blocked_created_at)
    payload.dedup_key = core.intake_candidate_delivery_dedup_key(payload.proposal_id, payload.effect_id, payload.effect_id)
    payload.reintake_effect_updated_at = blocked_created_at
    local base_version = expected_decision_key(payload)
    local blocked_version = conv_reconcile.reconcile_state_version(base_version, 3)
    local source_digest = convergence_shared.source_ref_digest(payload.source_ref)
    mock_bot_env()
    mock_intake_judge_view({ "fkst-dev:enabled", "fkst-dev:blocked" }, {
      m_builders.intake_decision_marker(payload.proposal_id, "enable", expected_decision_key(payload, command, blocked_created_at), "standard"),
      core.state_marker(payload.proposal_id, "thinking", base_version .. "/loop/3"),
      conv_rounds.converge_round_marker(payload.proposal_id, base_version, source_digest, 3, base_version .. "/loop/3", "Same narrowed question", {
        { angle = "minimal", verdict = "abstain", digest = "same-digest" },
      }),
      conv_reconcile.reconcile_marker(payload.proposal_id, base_version, 3, "drop"),
      trusted_comment(core.state_marker(payload.proposal_id, "blocked", blocked_version), blocked_created_at),
      command,
    })
    mock_intake_codex("⟦FKST:INTAKE⟧ enable\n⟦FKST:CLASS⟧ standard\n⟦FKST:REASON⟧ Reintake abandons the blocked framing and starts a fresh intake generation.")

    local result = run_judge(payload, opts("intake-reintake-terminal-blocked"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 4)
    local command_comment = find_comment_body(result.raises, "operator command accepted: reintake")
    local intake_comment = find_comment_body(result.raises, 'decision="enable"')
    local request = find_raise(result.raises, "github-devloop.devloop_execute_request").payload
    t.is_true(command_comment ~= nil)
    t.is_true(intake_comment ~= nil)
    t.is_true(intake_comment.body:find(expected_decision_key(payload, command, blocked_created_at), 1, true) ~= nil)
    t.eq(request.dedup_key, expected_decision_key(payload, command, blocked_created_at))
    t.eq(request.dedup_key == expected_decision_key(payload), false)
    assert_execution_request_chain(result.raises, payload, command)
    t.eq(count_calls("codex exec"), 1)
  end,

  test_judge_reintake_refuses_after_blocked_then_newer_active_marker_with_stale_labels = function()
    local command = trusted_reintake_command("IC_reintake_current_active")
    local payload = reintake_candidate(command)
    mock_bot_env()
    mock_intake_judge_view({ "fkst-dev:enabled", "fkst-dev:blocked" }, {
      m_builders.intake_decision_marker(payload.proposal_id, "decline", expected_decision_key(payload, command, command.created_at), "standard"),
      trusted_comment(core.state_marker(payload.proposal_id, "blocked", "github-devloop/issue/owner/repo/42/2026-06-04T01-00-00Z/intake/1"), "2026-06-04T01:00:00Z"),
      trusted_comment(core.state_marker(payload.proposal_id, "thinking", "github-devloop/issue/owner/repo/42/2026-06-04T02-00-00Z/intake/2"), "2026-06-04T02:00:00Z"),
      command,
    })

    local result = run_judge(payload, opts("intake-reintake-current-active-after-blocked"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local refusal = find_comment_body(result.raises, "operator command refused")
    t.is_true(refusal ~= nil)
    t.is_true(refusal.body:find("reintake requires terminal blocked or no active devloop state", 1, true) ~= nil)
    t.eq(count_calls("codex exec"), 0)
  end,

  test_judge_reintake_stale_candidate_is_skipped = function()
    local payload = candidate()
    local command = trusted_reintake_command("IC_reintake_stale")
    mock_bot_env()
    mock_intake_judge_view({}, {
      m_builders.intake_decision_marker(payload.proposal_id, "decline", expected_decision_key(payload), "standard"),
      command,
    })

    local result = run_judge(payload, opts("intake-reintake-stale-candidate"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
    t.eq(count_calls("codex exec"), 0)
  end,

  test_judge_reintake_mid_pipeline_refuses = function()
    local command = trusted_reintake_command("IC_reintake_judge_active")
    local payload = reintake_candidate(command)
    mock_bot_env()
    mock_intake_judge_view({ "fkst-dev:thinking" }, {
      m_builders.intake_decision_marker(payload.proposal_id, "decline", expected_decision_key(payload, command, command.created_at), "standard"),
      command,
    })

    local result = run_judge(payload, opts("intake-reintake-judge-active-state"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local refusal = find_comment_body(result.raises, "operator command refused")
    t.is_true(refusal ~= nil)
    t.is_true(refusal.body:find("reintake requires terminal blocked or no active devloop state", 1, true) ~= nil)
    t.is_true(refusal.body:find("use rereview, reready, or reimplement", 1, true) ~= nil)
    t.eq(count_calls("codex exec"), 0)
  end,
}
