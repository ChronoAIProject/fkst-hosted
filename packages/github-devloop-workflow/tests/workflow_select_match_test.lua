local core = require("core")
local base_ids = require("devloop.base_ids")
local blueprint = require("core.blueprint")
local digest = require("core.digest")
local devloop_base = require("devloop.base")
local devloop_facts = require("devloop.markers.facts")
local devloop_marker_builders = require("devloop.markers.builders")
local devloop_state = require("devloop.state")
local execution_start = require("devloop.execution_start")
local graph = require("testkit.graph")
local marker = require("core.marker")
local payloads_builders = require("devloop.payloads.builders")
local testing = require("testkit.testing")
local t = fkst.test

local candidate_queue = "github-devloop-intake.devloop_intake_candidate"

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

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
  for index, comment in ipairs(comments or {}) do
    local c = type(comment) == "table" and comment or { body = tostring(comment or "") }
    table.insert(rendered, string.format(
      '{"id":"%s","body":"%s","createdAt":"%s","author":{"login":"%s"}}',
      json_string(c.id or ("comment-" .. tostring(index))),
      json_string(c.body or ""),
      json_string(c.created_at or "2026-06-03T01:03:00Z"),
      json_string(c.author_login or "fkst-test-bot")
    ))
  end
  return table.concat(rendered, ",")
end

local function issue_view_stdout(fields)
  local f = fields or {}
  return string.format(
    '{"title":"%s","body":"%s","createdAt":"%s","updatedAt":"%s","state":"%s","labels":[%s],"comments":[%s],"assignees":[{"login":"%s"}],"author":{"login":"%s"}}\n',
    json_string(f.title or "Run the release workflow"),
    json_string(f.body or "Please run the release workflow for this repository."),
    json_string(f.created_at or "2026-06-03T01:00:00Z"),
    json_string(f.updated_at or "2026-06-03T01:02:03Z"),
    json_string(f.state or "OPEN"),
    encode_labels(f.labels or {}),
    encode_comments(f.comments or {}),
    json_string(f.assignee or "fkst-test-bot"),
    json_string(f.author_login or "fkst-test-bot")
  )
end

local function workflow_json(id, selector_json, step_intent)
  local selector = selector_json ~= nil and (',"selector":' .. selector_json) or ""
  return string.format(
    '{"schema":"fkst.workflow.v1","id":"%s","version":"2026-07-02","summary":"%s summary","applies_when":"%s applies to matching origin issues","steps":[{"id":"first","title":"First step","content":{"kind":"static","intent":"%s"}}]%s}',
    json_string(id),
    json_string(id),
    json_string(id),
    json_string(step_intent),
    selector
  )
end

local function test_root()
  local token = tostring({}):gsub("[^A-Za-z0-9]", "")
  return "/tmp/fkst-workflow-select-match-" .. token
end

local function cleanup(root)
  os.remove(root .. "/workflow-alpha.json")
  os.remove(root .. "/workflow-beta.json")
  os.execute("rmdir " .. shell_quote(root) .. " >/dev/null 2>&1")
end

local function mkdir_p(path)
  local ok = os.execute("mkdir -p " .. shell_quote(path))
  if ok ~= true and ok ~= 0 then
    error("failed to create temp workflow catalog")
  end
end

local function with_catalog(files, fn)
  local root = test_root()
  cleanup(root)
  mkdir_p(root)
  for name, source in pairs(files or {}) do
    file.write(root .. "/" .. name, source)
  end
  local ok, err = pcall(function()
    fn(root)
  end)
  cleanup(root)
  if not ok then
    error(err, 0)
  end
end

local function candidate()
  return payloads_builders.build_devloop_intake_candidate_payload("owner/repo", 42, "2026-06-03T01:02:03Z")
end

local function decision_key_for_current(payload, current)
  local c = current or {}
  return devloop_base.intake_decision_dedup_key(payload.proposal_id, {
    title = c.title or "Run the release workflow",
    body = c.body or "Please run the release workflow for this repository.",
  })
end

local function event(payload)
  return {
    queue = candidate_queue,
    payload = payload,
    ts = "2026-06-03T01:02:03Z",
  }
end

local function mock_env(root)
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
  t.mock_command('printf %s "$FKST_WORKFLOW_CATALOG_ROOT"', {
    stdout = root,
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_view(current, times)
  for _ = 1, times or 1 do
    t.mock_command("gh issue view", {
      stdout = issue_view_stdout(current),
      stderr = "",
      exit_code = 0,
    })
  end
end

local function mock_issue_views(...)
  for _, current in ipairs({ ... }) do
    mock_issue_view(current, 1)
  end
end

local function mock_workflow_codex(stdout, exit_code)
  t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
    stdout = "/tmp/fkst-packages-test/github-devloop-workflow/workflow-select-runtime",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("mkdir -p", { stdout = "", stderr = "", exit_code = 0 })
  t.mock_command("codex exec", {
    stdout = stdout,
    stderr = "",
    exit_code = exit_code or 0,
  })
end

local function mock_default_context_bundle(current)
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
  for _ = 1, 4 do
    t.mock_command('printf %s "$FKST_RUNTIME_ROOT"', {
      stdout = "/tmp/fkst-packages-test/github-devloop-workflow/default-intake-runtime",
      stderr = "",
      exit_code = 0,
    })
  end
  for _ = 1, 8 do
    t.mock_command("test -d", { stdout = "", stderr = "", exit_code = 1 })
    t.mock_command("test -e", { stdout = "", stderr = "", exit_code = 1 })
  end
  t.mock_command("install -d -m 0755", ok)
  t.mock_command("mktemp -d", {
    stdout = "/tmp/fkst-packages-test/github-devloop-workflow/default-intake-runtime/context/.bundle-tmp.intake\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue view", {
    stdout = issue_view_stdout(current),
    stderr = "",
    exit_code = 0,
  })
  t.mock_command("gh issue list", { stdout = "[]\n", stderr = "", exit_code = 0 })
  t.mock_command("gh pr list", { stdout = "[]\n", stderr = "", exit_code = 0 })
  for _ = 1, 8 do
    t.mock_command("touch ", ok)
    t.mock_command("printf %s '", ok)
    t.mock_command(" > ", ok)
    t.mock_command("test -r", ok)
    t.mock_command("wc -c < ", { stdout = "1\n", stderr = "", exit_code = 0 })
  end
  t.mock_command("python3 -c", ok)
  t.mock_command("rm -rf ", ok)
  t.mock_command("mkdir -p", ok)
end

local function mock_default_codex(stdout, current)
  t.mock_command('printf %s "$FKST_OUTPUT_LANG"', {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })
  mock_default_context_bundle(current)
  t.mock_command("codex exec", {
    stdout = stdout or "⟦FKST:INTAKE⟧ enable\n⟦FKST:CLASS⟧ standard\n⟦FKST:REASON⟧ Clear bounded implementation task.",
    stderr = "",
    exit_code = 0,
  })
end

local function run_workflow_select(payload)
  return testing.run_fake(require("departments.workflow_select.main"), event(payload))
end

local function raises_to_queue(raises, queue)
  local result = {}
  for _, raised in ipairs(raises or {}) do
    if raised.queue == queue then
      table.insert(result, raised)
    end
  end
  return result
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

local function assert_default_enable_raised(result)
  t.is_true(#raises_to_queue(result.raises, "github-devloop.devloop_execute_request") == 1)
  t.is_true(#raises_to_queue(result.raises, "github-proxy.github_issue_create_request") == 0)
  for _, raised in ipairs(result.raises) do
    if raised.queue == "github-proxy.github_issue_comment_request" then
      t.is_nil(raised.payload.body:find("github-devloop-workflow:blueprint:v1", 1, true))
    end
  end
end

local function assert_no_intake_marker_or_consensus(raises)
  t.eq(#raises_to_queue(raises, "consensus.proposal"), 0)
  for _, raised in ipairs(raises or {}) do
    t.is_true(raised.queue ~= "github-proxy.github_issue_comment_request"
      or tostring(raised.payload and raised.payload.body or ""):find("github-devloop:intake-decision:v1", 1, true) == nil)
  end
end

local function build_normal_enable_request(payload, current)
  return execution_start.build_execution_request_payload({
    proposal_id = payload.proposal_id,
    dedup_key = decision_key_for_current(payload, current),
    source_ref = payload.source_ref,
    origin = {
      package = "github-devloop-intake-default",
      route = "default",
      decision = "enable",
    },
    service_class = payload.service_class,
  })
end

local function ready_event_version_from_marker_version(marker_version)
  return base_ids.dedup_key({
    "ready",
    tostring(marker_version),
  })
end

local function first_raise_payload(result, queue)
  local found = raises_to_queue(result.raises, queue)
  return found[1] and found[1].payload or nil
end

local function lineage_header(origin, blueprint_digest, slot)
  local header, err = marker.build_lineage_header(origin, blueprint_digest or "d-1234567890", slot or "slot-one")
  t.is_nil(err)
  return header
end

local function run_fallthrough_case(root, current, workflow_stdout)
  local payload = candidate()
  mock_env(root)
  mock_issue_view(current, 2)
  if workflow_stdout ~= nil then
    mock_workflow_codex(workflow_stdout)
  end
  mock_default_codex(nil, current)
  local result = run_workflow_select(payload)
  assert_default_enable_raised(result)
  return result, codex_calls()
end

local function run_selected_workflow_case(root, first_current, fresh_current)
  local payload = candidate()
  mock_env(root)
  mock_issue_views(first_current, fresh_current or first_current)
  mock_workflow_codex("⟦FKST:WORKFLOW_SELECT⟧ workflow-alpha")
  return run_workflow_select(payload), payload
end

local function blueprint_comment(payload, workflow_id, plan_digest)
  return marker.build_blueprint_marker(payload.proposal_id, workflow_id, plan_digest)
end

local function intake_decision_comment(payload)
  return devloop_marker_builders.intake_decision_marker(payload.proposal_id, "track", decision_key_for_current(payload), "standard")
end

local tests = {
  test_trusted_workflow_child_fast_paths_to_execute_request_without_intake = function()
    local payload = candidate()
    local origin = "github-devloop/issue/owner/repo/7"
    local body = lineage_header(origin, "d-1234567890", "slot-one") .. "\n\nGenerated child spec body."
    local current = {
      body = body,
      author_login = "fkst-test-bot",
      labels = {},
    }

    mock_env("/tmp/fkst-packages-test/github-devloop-workflow/no-extra-catalog")
    mock_issue_view(current, 1)

    local result = run_workflow_select(payload)
    t.eq(#codex_calls(), 0)
    t.eq(#raises_to_queue(result.raises, "github-devloop.devloop_execute_request"), 1)
    t.eq(#raises_to_queue(result.raises, "github-proxy.github_issue_label_request"), 1)
    t.eq(#raises_to_queue(result.raises, "github-proxy.github_issue_comment_request"), 0)
    assert_no_intake_marker_or_consensus(result.raises)

    local request = first_raise_payload(result, "github-devloop.devloop_execute_request")
    t.eq(request.schema, "github-devloop.execution-request.v1")
    t.eq(request.proposal_id, payload.proposal_id)
    t.eq(request.source_ref.kind, "external")
    t.eq(request.source_ref.ref, "owner/repo#issue/42")
    local normal = build_normal_enable_request(payload, current)
    t.eq(request.dedup_key, normal.dedup_key)
    t.eq(request.dedup_key, decision_key_for_current(payload, current))
    t.is_true(request.dedup_key:find("workflow/child-execute", 1, true) == nil)
    t.eq(request.dedup_key:find("/intake/", 1, true) ~= nil, true)
    t.eq(request.origin.package, "github-devloop-workflow")
    t.eq(request.origin.route, "workflow-child")
    t.eq(request.origin.decision, "committed-child")
    t.eq(request.origin.lineage.origin, origin)
    t.eq(request.origin.lineage.blueprint_digest, "d-1234567890")
    t.eq(request.origin.lineage.slot, "slot-one")
    t.eq(request.service_class, "standard")
    t.is_nil(request.body)
    t.is_nil(request.content)

    local label = first_raise_payload(result, "github-proxy.github_issue_label_request")
    t.eq(label.source_ref.ref, "owner/repo#issue/42")
    t.eq(label.add_labels[1], "fkst-dev:enabled")
    t.eq(label.add_labels[2], "fkst-class:standard")

    local ready_version = "consensus:" .. tostring(request.dedup_key)
    t.eq(ready_event_version_from_marker_version(ready_version), "ready/consensus-" .. tostring(normal.dedup_key))
    t.eq(devloop_state.versioned_transition_status(
      { state = "ready", version = ready_version },
      { "ready" },
      "implementing",
      ready_version
    ), "apply")

    local live_current_ready_marker_version = "consensus:github-devloop/issue/owner/repo/191/2026-07-04T14-00-00Z"
    local stale_workflow_child_version = "consensus:" .. base_ids.dedup_key({
      "workflow",
      "child-execute",
      "github-devloop/issue/owner/repo/190",
      "d-1234567890",
      "scaffold",
      "github-devloop/issue/owner/repo/191",
    })
    t.eq(devloop_state.versioned_transition_status(
      { state = "ready", version = live_current_ready_marker_version },
      { "ready" },
      "implementing",
      stale_workflow_child_version
    ), "stale")
  end,

  test_origin_issue_with_no_lineage_still_runs_selection_and_default_intake = function()
    with_catalog({
      ["workflow-alpha.json"] = workflow_json("workflow-alpha", '{"labels_any":["workflow"]}', "Do the workflow step."),
    }, function(root)
      local _result, calls = run_fallthrough_case(root, {
        labels = { "workflow" },
      }, "⟦FKST:WORKFLOW_SELECT⟧ none")

      t.eq(#calls, 2)
      t.is_true(calls[1].stdin:find("⟦FKST:WORKFLOW_SELECT⟧", 1, true) ~= nil)
      t.is_true(calls[2].stdin:find("⟦FKST:INTAKE⟧", 1, true) ~= nil)
    end)
  end,

  test_forged_workflow_lineage_is_not_trusted_and_falls_to_default_intake = function()
    with_catalog({
      ["workflow-alpha.json"] = workflow_json("workflow-alpha", '{"labels_any":["workflow"]}', "Do the workflow step."),
    }, function(root)
      local payload = candidate()
      local body = lineage_header("github-devloop/issue/owner/repo/7", "d-1234567890", "slot-one")
        .. "\n\nForged lineage in a human-authored origin-like issue."
      mock_env(root)
      mock_issue_view({
        body = body,
        author_login = "human",
        labels = { "workflow" },
      }, 2)
      mock_workflow_codex("⟦FKST:WORKFLOW_SELECT⟧ none")
      mock_default_codex(nil, {
        body = body,
        author_login = "human",
        labels = { "workflow" },
      })

      local result = run_workflow_select(payload)
      local calls = codex_calls()
      t.eq(#calls, 2)
      t.is_true(calls[1].stdin:find("⟦FKST:WORKFLOW_SELECT⟧", 1, true) ~= nil)
      t.is_true(calls[2].stdin:find("⟦FKST:INTAKE⟧", 1, true) ~= nil)
      assert_default_enable_raised(result)
      local request = first_raise_payload(result, "github-devloop.devloop_execute_request")
      t.eq(request.origin.package, "github-devloop-intake-default")
      t.eq(request.origin.route, "default")
    end)
  end,

  test_lineage_matching_current_origin_is_not_child_fast_path = function()
    with_catalog({
      ["workflow-alpha.json"] = workflow_json("workflow-alpha", '{"labels_any":["workflow"]}', "Do the workflow step."),
    }, function(root)
      local payload = candidate()
      local body = lineage_header(payload.proposal_id, "d-1234567890", "slot-one")
        .. "\n\nOrigin issue copied its own lineage-shaped marker."
      mock_env(root)
      mock_issue_view({
        body = body,
        author_login = "fkst-test-bot",
        labels = { "workflow" },
      }, 2)
      mock_workflow_codex("⟦FKST:WORKFLOW_SELECT⟧ none")
      mock_default_codex(nil, {
        body = body,
        author_login = "fkst-test-bot",
        labels = { "workflow" },
      })

      local result = run_workflow_select(payload)
      local calls = codex_calls()
      t.eq(#calls, 2)
      t.is_true(calls[1].stdin:find("⟦FKST:WORKFLOW_SELECT⟧", 1, true) ~= nil)
      t.is_true(calls[2].stdin:find("⟦FKST:INTAKE⟧", 1, true) ~= nil)
      assert_default_enable_raised(result)
      local request = first_raise_payload(result, "github-devloop.devloop_execute_request")
      t.eq(request.origin.package, "github-devloop-intake-default")
    end)
  end,

  test_selector_match_writes_one_blueprint_track_decision_without_default_or_child = function()
    local source = workflow_json("workflow-alpha", '{"labels_any":["workflow"]}', "SECRET STEP BODY MUST NOT ENTER PROMPT OR PAYLOAD")
    with_catalog({
      ["workflow-alpha.json"] = source,
    }, function(root)
      local payload = candidate()
      local parsed = blueprint.parse_blueprint(source)
      local plan_digest = digest.blueprint_digest(parsed)

      mock_env(root)
      mock_issue_view({ labels = { "workflow" } }, 2)
      mock_workflow_codex("⟦FKST:WORKFLOW_SELECT⟧ workflow-alpha")

      local result = run_workflow_select(payload)
      t.eq(#result.raises, 1)
      t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
      t.eq(#raises_to_queue(result.raises, "github-devloop.devloop_execute_request"), 0)
      t.eq(#raises_to_queue(result.raises, "github-proxy.github_issue_create_request"), 0)
      t.eq(#raises_to_queue(result.raises, "github-proxy.github_issue_label_request"), 0)

      local request = result.raises[1].payload
      t.eq(request.dedup_key, base_ids.dedup_key({
        "workflow",
        "blueprint-decision",
        tostring(payload.proposal_id),
        tostring(payload.dedup_key),
      }))
      local blueprint_marker = marker.parse_blueprint_marker(request.body, payload.proposal_id)
      t.eq(blueprint_marker.workflow, "workflow-alpha")
      t.eq(blueprint_marker.digest, plan_digest)

      local intake = devloop_facts.intake_decision_fact({
        { body = request.body, author_login = "fkst-test-bot", created_at = "2026-07-03T00:00:00Z" },
      }, payload.proposal_id)
      t.eq(intake.decision, "track")
      t.eq(intake.dedup_key, payload.dedup_key)
      t.is_nil(request.body:find("SECRET STEP BODY", 1, true))

      local calls = codex_calls()
      t.eq(#calls, 1)
      t.is_true(calls[1].stdin:find("⟦FKST:WORKFLOW_SELECT⟧", 1, true) ~= nil)
      t.is_nil(calls[1].stdin:find("⟦FKST:INTAKE⟧", 1, true))
      t.is_true(calls[1].stdin:find("workflow-alpha summary", 1, true) ~= nil)
      t.is_nil(calls[1].stdin:find("SECRET STEP BODY", 1, true))
    end)
  end,

  test_non_english_title_without_selector_keyword_can_select_semantic_builtin_feature_workflow = function()
    local payload = candidate()
    mock_env("/tmp/fkst-packages-test/github-devloop-workflow/no-extra-catalog")
    mock_issue_view({
      title = "添加导出功能",
      body = "Please implement a new bounded CSV export capability for the reports page as one end-to-end slice with a visible button, export endpoint, and acceptance test.",
      labels = { "feature" },
    }, 2)
    mock_workflow_codex("⟦FKST:WORKFLOW_SELECT⟧ software-feature-flow")

    local result = run_workflow_select(payload)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.eq(#raises_to_queue(result.raises, "github-devloop.devloop_execute_request"), 0)

    local request = result.raises[1].payload
    local blueprint_marker = marker.parse_blueprint_marker(request.body, payload.proposal_id)
    t.eq(blueprint_marker.workflow, "software-feature-flow")

    local calls = codex_calls()
    t.eq(#calls, 1)
    t.is_true(calls[1].stdin:find("⟦FKST:WORKFLOW_SELECT⟧", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("software-feature-flow", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("new bounded software capability", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("添加导出功能", 1, true) ~= nil)
    t.is_nil(calls[1].stdin:find("software-dev-flow", 1, true))
  end,

  test_already_clear_work_falls_through_and_prompt_carries_goal_clarity_gate = function()
    local _result, calls = run_fallthrough_case(
      "/tmp/fkst-packages-test/github-devloop-workflow/no-extra-catalog",
      {
        title = "Settings spinner never stops after saving",
        body = "The settings save button leaves the page spinner running forever after the save request returns. Please fix the local settings save flow so the spinner stops on success and add a regression test.",
        labels = { "bug" },
      },
      "⟦FKST:WORKFLOW_SELECT⟧ none"
    )

    t.eq(#calls, 2)
    t.is_true(calls[1].stdin:find("⟦FKST:WORKFLOW_SELECT⟧", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("idea-to-goal-flow", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("is the GOAL / OBJECTIVE clear", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("OBJECTIVE itself is genuinely fuzzy/unformed", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("If a concrete OBJECTIVE is already clear", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("even a symptom bug whose fix location is unknown", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("Well-specified feature -> software-feature-flow", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("plain devloop", 1, true) ~= nil)
    t.is_nil(calls[1].stdin:find("software-diagnose-plan-flow", 1, true))
    t.is_true(calls[2].stdin:find("⟦FKST:INTAKE⟧", 1, true) ~= nil)
  end,

  test_ordinary_symptom_bug_falls_to_plain_devloop_and_prompt_carries_objective_clarity_rule = function()
    local _result, calls = run_fallthrough_case(
      "/tmp/fkst-packages-test/github-devloop-workflow/no-extra-catalog",
      {
        title = "Login button returns 500",
        body = "Login button returns 500 on submit, please fix",
        labels = { "bug" },
      },
      "⟦FKST:WORKFLOW_SELECT⟧ none"
    )

    t.eq(#calls, 2)
    t.is_true(calls[1].stdin:find("⟦FKST:WORKFLOW_SELECT⟧", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("idea-to-goal-flow", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("is the GOAL / OBJECTIVE clear", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("Login button returns 500 on submit, please fix", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("OBJECTIVE is clear", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("stop the 500", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("fix location is unknown", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("Ordinary bug reports, even symptom-only ones, go to plain devloop", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("OBJECTIVE itself is genuinely fuzzy/unformed", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("When in doubt, choose plain devloop", 1, true) ~= nil)
    t.is_true(calls[2].stdin:find("⟦FKST:INTAKE⟧", 1, true) ~= nil)
  end,

  test_vague_goalless_idea_can_select_idea_to_goal_flow = function()
    local payload = candidate()
    mock_env("/tmp/fkst-packages-test/github-devloop-workflow/no-extra-catalog")
    mock_issue_view({
      title = "Make workflow ideas less fuzzy",
      body = "I keep dropping half-formed workflow thoughts into issues and the bot jumps straight to code. There should be some way to turn the thought into the real goal first, but I do not know what exact change or acceptance should be.",
      labels = { "idea" },
    }, 2)
    mock_workflow_codex("⟦FKST:WORKFLOW_SELECT⟧ idea-to-goal-flow")

    local result = run_workflow_select(payload)
    t.eq(#result.raises, 1)
    t.eq(result.raises[1].queue, "github-proxy.github_issue_comment_request")
    t.eq(#raises_to_queue(result.raises, "github-devloop.devloop_execute_request"), 0)

    local request = result.raises[1].payload
    local blueprint_marker = marker.parse_blueprint_marker(request.body, payload.proposal_id)
    t.eq(blueprint_marker.workflow, "idea-to-goal-flow")

    local calls = codex_calls()
    t.eq(#calls, 1)
    t.is_true(calls[1].stdin:find("⟦FKST:WORKFLOW_SELECT⟧", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("idea-to-goal-flow", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("fuzzy raw idea, exploration, or open-ended wish", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("one concrete, code-verifiable objective", 1, true) ~= nil)
    t.is_true(calls[1].stdin:find("OBJECTIVE itself is genuinely fuzzy/unformed", 1, true) ~= nil)
  end,

  test_workflow_selection_skips_blueprint_when_fresh_issue_is_closed = function()
    with_catalog({
      ["workflow-alpha.json"] = workflow_json("workflow-alpha", '{"labels_any":["workflow"]}', "Do the workflow step."),
    }, function(root)
      local result = run_selected_workflow_case(root, {
        labels = { "workflow" },
      }, {
        labels = { "workflow" },
        state = "CLOSED",
      })

      t.eq(#raises_to_queue(result.raises, "github-proxy.github_issue_comment_request"), 0)
      t.eq(#raises_to_queue(result.raises, "github-devloop.devloop_execute_request"), 0)
      t.eq(#codex_calls(), 1)
    end)
  end,

  test_workflow_selection_skips_blueprint_when_fresh_decision_dedup_changes = function()
    with_catalog({
      ["workflow-alpha.json"] = workflow_json("workflow-alpha", '{"labels_any":["workflow"]}', "Do the workflow step."),
    }, function(root)
      local result = run_selected_workflow_case(root, {
        labels = { "workflow" },
        body = "Original content that selected the workflow.",
      }, {
        labels = { "workflow" },
        body = "Changed content makes the slow workflow selection stale.",
        updated_at = "2026-06-03T01:04:00Z",
      })

      t.eq(#raises_to_queue(result.raises, "github-proxy.github_issue_comment_request"), 0)
      t.eq(#raises_to_queue(result.raises, "github-devloop.devloop_execute_request"), 0)
      t.eq(#codex_calls(), 1)
    end)
  end,

  test_workflow_selection_skips_blueprint_when_fresh_intake_decision_exists = function()
    with_catalog({
      ["workflow-alpha.json"] = workflow_json("workflow-alpha", '{"labels_any":["workflow"]}', "Do the workflow step."),
    }, function(root)
      local payload = candidate()
      mock_env(root)
      mock_issue_views({
        labels = { "workflow" },
      }, {
        labels = { "workflow" },
        comments = {
          {
            body = intake_decision_comment(payload),
            author_login = "fkst-test-bot",
          },
        },
      })
      mock_workflow_codex("⟦FKST:WORKFLOW_SELECT⟧ workflow-alpha")

      local result = run_workflow_select(payload)
      t.eq(#raises_to_queue(result.raises, "github-proxy.github_issue_comment_request"), 0)
      t.eq(#raises_to_queue(result.raises, "github-devloop.devloop_execute_request"), 0)
      t.eq(#codex_calls(), 1)
    end)
  end,

  test_workflow_selection_skips_duplicate_when_fresh_blueprint_exists = function()
    local source = workflow_json("workflow-alpha", '{"labels_any":["workflow"]}', "Do the workflow step.")
    with_catalog({
      ["workflow-alpha.json"] = source,
    }, function(root)
      local payload = candidate()
      local parsed = blueprint.parse_blueprint(source)
      local plan_digest = digest.blueprint_digest(parsed)
      mock_env(root)
      mock_issue_views({
        labels = { "workflow" },
      }, {
        labels = { "workflow" },
        comments = {
          {
            body = blueprint_comment(payload, "workflow-alpha", plan_digest),
            author_login = "fkst-test-bot",
          },
        },
      })
      mock_workflow_codex("⟦FKST:WORKFLOW_SELECT⟧ workflow-alpha")

      local result = run_workflow_select(payload)
      t.eq(#raises_to_queue(result.raises, "github-proxy.github_issue_comment_request"), 0)
      t.eq(#raises_to_queue(result.raises, "github-devloop.devloop_execute_request"), 0)
      t.eq(#codex_calls(), 1)
    end)
  end,

  test_selector_no_match_with_bounded_catalog_defers_to_workflow_judge_none = function()
    with_catalog({
      ["workflow-alpha.json"] = workflow_json("workflow-alpha", '{"labels_any":["workflow"],"title_contains_any":["workflow"]}', "Do the workflow step."),
    }, function(root)
      local _result, calls = run_fallthrough_case(root, {
        title = "Repair ordinary retry backoff",
        labels = { "bug" },
      }, "⟦FKST:WORKFLOW_SELECT⟧ none")

      t.eq(#calls, 2)
      t.is_true(calls[1].stdin:find("⟦FKST:WORKFLOW_SELECT⟧", 1, true) ~= nil)
      t.is_true(calls[1].stdin:find("workflow-alpha summary", 1, true) ~= nil)
      t.is_true(calls[2].stdin:find("⟦FKST:INTAKE⟧", 1, true) ~= nil)
    end)
  end,

  test_workflow_codex_none_falls_through_to_default_intake = function()
    with_catalog({
      ["workflow-alpha.json"] = workflow_json("workflow-alpha", '{"labels_any":["workflow"]}', "Do the workflow step."),
    }, function(root)
      local _result, calls = run_fallthrough_case(root, {
        labels = { "workflow" },
      }, "⟦FKST:WORKFLOW_SELECT⟧ none")

      t.eq(#calls, 2)
      t.is_true(calls[1].stdin:find("⟦FKST:WORKFLOW_SELECT⟧", 1, true) ~= nil)
      t.is_true(calls[2].stdin:find("⟦FKST:INTAKE⟧", 1, true) ~= nil)
    end)
  end,

  test_workflow_codex_unknown_id_falls_through_to_default_intake = function()
    with_catalog({
      ["workflow-alpha.json"] = workflow_json("workflow-alpha", '{"labels_any":["workflow"]}', "Do the matching workflow step."),
      ["workflow-beta.json"] = workflow_json("workflow-beta", '{"labels_any":["other"]}', "Do another workflow step."),
    }, function(root)
      local _result, calls = run_fallthrough_case(root, {
        labels = { "workflow" },
      }, "⟦FKST:WORKFLOW_SELECT⟧ workflow-gamma")

      t.eq(#calls, 2)
      t.is_true(calls[1].stdin:find("workflow-alpha summary", 1, true) ~= nil)
      t.is_true(calls[1].stdin:find("workflow-beta summary", 1, true) ~= nil)
      t.is_true(calls[2].stdin:find("⟦FKST:INTAKE⟧", 1, true) ~= nil)
    end)
  end,

  test_workflow_codex_garbage_falls_through_to_default_intake = function()
    with_catalog({
      ["workflow-alpha.json"] = workflow_json("workflow-alpha", '{"labels_any":["workflow"]}', "Do the workflow step."),
    }, function(root)
      local _result, calls = run_fallthrough_case(root, {
        labels = { "workflow" },
      }, "not a valid workflow id")

      t.eq(#calls, 2)
      t.is_true(calls[1].stdin:find("⟦FKST:WORKFLOW_SELECT⟧", 1, true) ~= nil)
      t.is_true(calls[2].stdin:find("⟦FKST:INTAKE⟧", 1, true) ~= nil)
    end)
  end,
}

return tests
