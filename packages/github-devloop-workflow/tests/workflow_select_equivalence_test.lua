local core = require("core")
local base_ids = require("devloop.base_ids")
local default_intake = require("core.default_intake")
local payloads_builders = require("devloop.payloads.builders")
local saga = require("workflow.saga")
local testing = require("testkit.testing")
local t = fkst.test
local author_policy = require("testkit.github_author_policy")

local candidate_queue = "github-devloop-intake.devloop_intake_candidate"

local function json_string(value)
  return tostring(value or "")
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\b", "\\b")
    :gsub("\f", "\\f")
    :gsub("\n", "\\n")
    :gsub("\r", "\\r")
    :gsub("\t", "\\t")
    :gsub("[%z\1-\31]", function(char)
      return string.format("\\u%04X", string.byte(char))
    end)
end

local function encode_labels(labels)
  local rendered = {}
  for _, label in ipairs(labels or {}) do
    table.insert(rendered, '{"name":"' .. json_string(label) .. '"}')
  end
  return table.concat(rendered, ",")
end

local function encode_comments(comments)
  local rendered = {}
  for _, comment in ipairs(comments or {}) do
    local body = type(comment) == "table" and comment.body or comment
    local author = type(comment) == "table" and comment.author_login or "fkst-test-bot"
    local created_at = type(comment) == "table" and comment.created_at or "2026-06-03T01:00:00Z"
    table.insert(rendered, '{"body":"' .. json_string(body)
      .. '","author":{"login":"' .. json_string(author)
      .. '"},"createdAt":"' .. json_string(created_at) .. '"}')
  end
  return table.concat(rendered, ",")
end

local function issue_view_stdout(fields)
  local f = fields or {}
  return string.format(
    '{"title":"%s","body":"%s","createdAt":"%s","updatedAt":"%s","state":"%s","labels":[%s],"comments":[%s],"assignees":[{"login":"%s"}],"author":{"login":"%s"}}\n',
    json_string(f.title or "Repair retry backoff for failed widget sync"),
    json_string(f.body or "Implement exponential backoff for widget sync retries. Acceptance: unit tests cover 1s, 2s, and capped retries."),
    json_string(f.created_at or "2026-06-03T01:00:00Z"),
    json_string(f.updated_at or "2026-06-03T01:02:03Z"),
    json_string(f.state or "OPEN"),
    encode_labels(f.labels),
    encode_comments(f.comments),
    json_string(f.assignee or "fkst-test-bot"),
    json_string(f.author_login or "fkst-test-bot")
  )
end

local function issue_list_stdout(issues)
  local rendered = {}
  for _, issue in ipairs(issues or {}) do
    table.insert(rendered, string.format(
      '{"number":%d,"title":"%s","body":"%s","updatedAt":"%s","labels":[%s],"assignees":[{"login":"%s"}],"author":{"login":"%s"},"closedAt":"%s"}',
      tonumber(issue.number) or 1,
      json_string(issue.title or "Issue"),
      json_string(issue.body or ""),
      json_string(issue.updated_at or "2026-06-03T01:02:03Z"),
      encode_labels(issue.labels or {}),
      json_string(issue.assignee or "fkst-test-bot"),
      json_string(issue.author_login or "fkst-test-bot"),
      json_string(issue.closed_at or "2026-06-02T01:02:03Z")
    ))
  end
  return "[" .. table.concat(rendered, ",") .. "]\n"
end

local function mock_env()
  t.mock_command('printf %s "$FKST_GITHUB_BOT_LOGIN"', {
    stdout = "fkst-test-bot",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_GITHUB_WRITE"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_OUTPUT_LANG"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command('printf %s "$FKST_WORKFLOW_CATALOG_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop-workflow/no-catalog",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_view(current, times)
  local result = {
    stdout = issue_view_stdout(current),
    stderr = "",
    exit_code = 0,
  }
  for _ = 1, times or 2 do
    t.mock_command("gh issue view", result)
  end
end

local function mock_context_bundle(current)
  local ok = { stdout = "", stderr = "", exit_code = 0 }
  author_policy.mock_env(t, {
    env = {
      FKST_DEVLOOP_MANAGED_BOT_LOGINS = "",
      FKST_GITHUB_AUTHORIZED_LOGINS = "",
    },
  }, {
    times = 4,
  })
  for _ = 1, 4 do
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop-workflow/runtime",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 8 do
    t.mock_command("test -d", { stdout = "", stderr = "", exit_code = 1 })
  end
  for _ = 1, 8 do
    t.mock_command("test -e", { stdout = "", stderr = "", exit_code = 1 })
  end
  t.mock_command("install -d -m 0755", ok)
  t.mock_command("mktemp -d", {
    stdout = "/tmp/fkst-packages-test/github-devloop-workflow/runtime/context/.bundle-tmp.intake\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue view", {
    stdout = issue_view_stdout(current),
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue list", {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh pr list", {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
  for _ = 1, 8 do
    t.mock_command("touch ", ok)
  end
  for _ = 1, 8 do
    t.mock_command("printf %s '", ok)
    t.mock_command(" > ", ok)
  end
  t.mock_command("python3 -c", ok)
  t.mock_command("rm -rf ", ok)
  for _ = 1, 8 do
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
end

local function mock_codex(stdout, current)
  mock_context_bundle(current)
  t.mock_command("codex exec", {
    stdout = stdout,
    stderr = "",
    exit_code = 0,
  })
end

local function mock_workflow_none()
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop-workflow/runtime",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("mkdir -p", {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("codex exec", {
    stdout = "⟦FKST:WORKFLOW_SELECT⟧ none",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_class_escalation_lists(siblings)
  t.mock_command("gh issue list", {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue list", {
    stdout = issue_list_stdout(siblings),
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue list", {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_workflow_select_path(case, current)
  mock_env()
  mock_issue_view(current, 2)
  mock_workflow_none()
  mock_codex(case.codex, current)
  if case.class_siblings ~= nil then
    mock_class_escalation_lists(case.class_siblings)
  end
end

local function mock_default_path(case, current)
  mock_env()
  mock_issue_view(current, 2)
  mock_codex(case.codex, current)
  if case.class_siblings ~= nil then
    mock_class_escalation_lists(case.class_siblings)
  end
end

local function candidate()
  return payloads_builders.build_devloop_intake_candidate_payload("owner/repo", 42, "2026-06-03T01:02:03Z")
end

local spec = {
  consumes = { candidate_queue },
  produces = {
    "github-devloop.devloop_execute_request",
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_create_request",
    "github-proxy.github_issue_label_request",
    "github-proxy.github_pr_comment_request",
  },
  stall_window = "2m",
}

local intake_judge_equivalent = saga.department(spec, {
  done = function(_event) return false end,
  act = function(event)
    return default_intake.act(core, event, { dept = "intake_judge" })
  end,
  wrap = core.wrap_pipeline_failure,
  name = "intake_judge",
})

local function event(payload)
  return {
    queue = candidate_queue,
    payload = payload,
    ts = "2026-06-03T01:02:03Z",
  }
end

local function run_workflow_select(payload, name)
  local _ = name
  return testing.run_fake(require("departments.workflow_select.main"), event(payload))
end

local function run_intake_judge_equivalent(payload)
  return testing.run_fake(intake_judge_equivalent, event(payload))
end

local function normalized_raises(raises)
  local normalized = {}
  for index, raised in ipairs(raises or {}) do
    normalized[index] = {
      queue = raised.queue,
      payload = raised.payload,
    }
  end
  return normalized
end

local function canonical(value)
  if value == nil then
    return "null"
  end
  if type(value) == "boolean" or type(value) == "number" then
    return tostring(value)
  end
  if type(value) == "string" then
    return string.format("%q", value)
  end
  local keys = {}
  for key in pairs(value) do
    table.insert(keys, key)
  end
  table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
  local parts = {}
  for _, key in ipairs(keys) do
    table.insert(parts, "[" .. canonical(key) .. "]=" .. canonical(value[key]))
  end
  return "{" .. table.concat(parts, ",") .. "}"
end

local function assert_same_raises(left, right)
  t.eq(canonical(normalized_raises(left)), canonical(normalized_raises(right)))
end

local function exercise_pair(case)
  local payload = candidate()
  local current = case.current or {}
  mock_workflow_select_path(case, current)
  local workflow_result = run_workflow_select(payload, "workflow-select-" .. case.name)

  mock_default_path(case, current)
  local judge_result = run_intake_judge_equivalent(payload)

  assert_same_raises(workflow_result.raises, judge_result.raises)
end

local class_siblings = {
  { number = 80, title = "Widget sync retry patch", labels = { "fingerprint:widget-sync" } },
  { number = 81, title = "Widget sync retry overflow fix", labels = { "fingerprint:widget-sync" } },
  { number = 82, title = "Widget sync timeout fix", labels = { "fingerprint:widget-sync" } },
}

local tests = {
  test_non_workflow_enable_matches_intake_judge = function()
    exercise_pair({
      name = "enable",
      codex = "⟦FKST:INTAKE⟧ enable\n⟦FKST:CLASS⟧ expedite\n⟦FKST:REASON⟧ Clear bounded implementation task.",
    })
  end,

  test_non_workflow_track_matches_intake_judge = function()
    exercise_pair({
      name = "track",
      codex = "⟦FKST:INTAKE⟧ track\n⟦FKST:CLASS⟧ background\n⟦FKST:REASON⟧ Umbrella tracker issue; individual waves should be separate proposals.",
    })
  end,

  test_non_workflow_decline_matches_intake_judge = function()
    exercise_pair({
      name = "decline",
      current = {
        body = "Rotate production credentials after human confirmation.",
        labels = { "fkst-class:background" },
      },
      codex = "⟦FKST:INTAKE⟧ decline\n⟦FKST:CLASS⟧ standard\n⟦FKST:REASON⟧ Requires production credentials and human confirmation.",
    })
  end,

  test_non_workflow_escalate_to_class_matches_intake_judge = function()
    exercise_pair({
      name = "escalate",
      current = {
        title = "Fix widget sync retry overflow again",
        body = "Third recurrence after #80 and #81; decide whether this needs a class-level retry policy.",
      },
      class_siblings = class_siblings,
      codex = "⟦FKST:INTAKE⟧ escalate-to-class\n⟦FKST:CLASS⟧ standard\n⟦FKST:REASON⟧ Cites #80 and #81 as prior siblings; Rule of Three requires class-level retry policy.",
    })
  end,
}

return tests
