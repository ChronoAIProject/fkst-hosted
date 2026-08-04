local payloads_builders = require("devloop.payloads.builders")
local testing = require("testkit.testing")
local t = fkst.test

local CANDIDATE_QUEUE = "github-devloop-intake.devloop_intake_candidate"
local BLOCKED_MARKER = '[fkst:blocked-github-content:v1 field="%s" existed="true" author_login="mallory" why="non-whitelisted-author"]'
local BLOCKED_REASON = "Mandatory origin title or body is unavailable because GitHub content was blocked; intake terminated before workflow selection or materialization."

local function json_string(value)
  return tostring(value or "")
    :gsub("\\", "\\\\")
    :gsub('"', '\\"')
    :gsub("\n", "\\n")
    :gsub("\r", "\\r")
    :gsub("\t", "\\t")
end

local function issue_view_stdout(current)
  return string.format(
    '{"title":"%s","body":"%s","createdAt":"2026-08-04T01:00:00Z","updatedAt":"2026-08-04T01:02:03Z","state":"OPEN","labels":[],"comments":[],"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
    json_string(current.title),
    json_string(current.body)
  )
end

local function candidate()
  return payloads_builders.build_devloop_intake_candidate_payload(
    "owner/repo",
    42,
    "2026-08-04T01:02:03Z"
  )
end

local function event(payload)
  return {
    queue = CANDIDATE_QUEUE,
    payload = payload,
    ts = "2026-08-04T01:02:03Z",
  }
end

local function mock_inputs(current)
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
  for _ = 1, 2 do
    t.mock_command("gh issue view", {
      stdout = issue_view_stdout(current),
      stderr = "",
      exit_code = 0,
    })
  end
end

local function raises_to_queue(raises, queue)
  local matches = {}
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue then
      matches[#matches + 1] = raised
    end
  end
  return matches
end

local function codex_call_count()
  local count = 0
  for _, call in ipairs(t.command_calls()) do
    if call.rendered:find("codex exec", 1, true) ~= nil then
      count = count + 1
    end
  end
  return count
end

local function assert_blocked_origin_declines(current)
  mock_inputs(current)
  local result = testing.run_fake(
    require("departments.workflow_select.main"),
    event(candidate())
  )

  t.eq(codex_call_count(), 0)
  t.eq(#raises_to_queue(result.raises, "github-proxy.github_issue_create_request"), 0)
  t.eq(#raises_to_queue(result.raises, "github-devloop.devloop_execute_request"), 0)

  local comments = raises_to_queue(result.raises, "github-proxy.github_issue_comment_request")
  t.eq(#comments, 1)
  t.is_true(comments[1].payload.body:find('decision="decline"', 1, true) ~= nil)
  t.is_true(comments[1].payload.body:find(BLOCKED_REASON, 1, true) ~= nil)
end

return {
  test_blocked_title_declines_before_workflow_selection = function()
    assert_blocked_origin_declines({
      title = string.format(BLOCKED_MARKER, "title"),
      body = "Readable body.",
    })
  end,

  test_blocked_body_declines_before_workflow_selection = function()
    assert_blocked_origin_declines({
      title = "Readable title",
      body = string.format(BLOCKED_MARKER, "body"),
    })
  end,

  test_blocked_title_and_body_decline_before_workflow_selection = function()
    assert_blocked_origin_declines({
      title = string.format(BLOCKED_MARKER, "title"),
      body = string.format(BLOCKED_MARKER, "body"),
    })
  end,
}
