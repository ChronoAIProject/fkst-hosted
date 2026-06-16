local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local opts = h.opts
local issue = h.issue
local reviewing = h.reviewing
local mock_issue_state = h.mock_issue_state
local run_observe = h.run_observe
local run_observe_pr = h.run_observe_pr
local find_raise = h.find_raise
local render_comment = h.render_comment
local json_string = h.json_string
local ready = h.ready
local run_implement = h.run_implement
local mock_issue_implement = h.mock_issue_implement
local deterministic_branch_for = h.deterministic_branch_for
local mock_fresh_implement_worktree = h.mock_fresh_implement_worktree
local mock_implement_codex = h.mock_implement_codex
local mock_git_status = h.mock_git_status
local mock_git_commit = h.mock_git_commit
local count_calls = h.count_calls
local entity_read_mocks = require("tests.entity_read_mock_helpers")

local repo = "owner/repo"
local proposal_id = "github-devloop/issue/owner/repo/42"
local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"

local function run_liveness_scan(name)
  return t.run_department("departments/liveness_scan/main.lua", {
    queue = "devloop_liveness_tick",
    payload = {
      schema = "github-devloop.tick.v1",
    },
    ts = "2026-06-03T01:32:03Z",
  }, opts(name or "liveness-scan"))
end

local function run_liveness_scan_at(name, ts, run_opts)
  return t.run_department("departments/liveness_scan/main.lua", {
    queue = "devloop_liveness_tick",
    payload = {
      schema = "github-devloop.tick.v1",
    },
    ts = ts,
  }, run_opts or opts(name or "liveness-scan"))
end

local function mock_repo()
  t.mock_command(core.read_env_command("FKST_GITHUB_REPO"), {
    stdout = repo,
    stderr = "",
    exit_code = 0,
  })
end

local function numbered_list_json(items)
  local rendered = {}
  for _, item in ipairs(items or {}) do
    table.insert(rendered, string.format(
      '{"number":%d,"state":"%s","updated_at":"%s"}',
      tonumber(item.number),
      json_string(item.state or "open"),
      json_string(item.updated_at or "")
    ))
  end
  return "[" .. table.concat(rendered, ",") .. "]\n"
end

local function mock_issue_list(items)
  t.mock_command(core.gh_issue_list_observe_cmd(repo), {
    stdout = numbered_list_json(items),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_issue_state_number(issue_number, labels, state, comments)
  entity_read_mocks.mock_issue_read_forms(t, {
    repo = repo,
    number = issue_number,
    title = "Issue " .. tostring(issue_number),
    body = "",
    state = state or "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    labels = labels,
    comments = comments,
    assignees = { "fkst-test-bot" },
    times = 1,
  })
end

local function mock_empty_pr_list()
  t.mock_command(core.gh_pr_list_observe_cmd(repo), {
    stdout = "[]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_list(items)
  t.mock_command(core.gh_pr_list_observe_cmd(repo), {
    stdout = numbered_list_json(items),
    stderr = "",
    exit_code = 0,
  })
end

local function mock_pr_state(comments, state)
  entity_read_mocks.mock_pr_read_forms(t, {
    repo = repo,
    number = 7,
    head = "devloop-owner-repo-42-01HY",
    head_sha = "def456",
    base_branch = "dev",
    state = state or "OPEN",
    updated_at = "2026-06-04T01:02:03Z",
    comments = comments,
    times = 1,
  })
end

local function mock_linked_pr_state(comments, state, exit_code, times)
  local rendered = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered, render_comment(comment))
  end
  local stderr = ""
  if exit_code ~= nil and exit_code ~= 0 then
    stderr = "pr view failed"
  end
  entity_read_mocks.mock_pr_view_raw_selector(t, { repo = repo, number = 7 }, entity_read_mocks.pr_origin_selector, {
    stdout = string.format(
      '{"headRefName":"devloop-owner-repo-42-01HY","headRefOid":"def456","baseRefName":"dev","state":"%s","updatedAt":"2026-06-04T01:02:03Z","comments":[%s]}\n',
      json_string(state or "OPEN"),
      table.concat(rendered, ",")
    ),
    stderr = stderr,
    exit_code = exit_code or 0,
  }, times or 1)
  if exit_code == nil or exit_code == 0 then
    entity_read_mocks.mock_pr_read_forms(t, {
      repo = repo,
      number = 7,
      head = "devloop-owner-repo-42-01HY",
      head_sha = "def456",
      base_branch = "dev",
      state = state or "OPEN",
      updated_at = "2026-06-04T01:02:03Z",
      comments = comments,
      times = times or 1,
    })
  end
end

local function mock_linked_pr_absent(times)
  entity_read_mocks.mock_pr_view_raw_selector(t, { repo = repo, number = 7 }, entity_read_mocks.pr_origin_selector, {
    stdout = "",
    stderr = "HTTP 404: Not Found",
    exit_code = 1,
  }, times or 1)
end

local function assert_no_entity_change(result)
  t.eq(result.exit_code, 0)
  t.eq(find_raise(result.raises, "github-proxy.github_entity_changed"), nil)
end

local function entity_change_issue_numbers(result)
  local numbers = {}
  for _, raised in ipairs(result.raises or {}) do
    if raised.queue == "github-proxy.github_entity_changed"
      and raised.payload ~= nil
      and raised.payload.type == "issue" then
      numbers[tonumber(raised.payload.number)] = true
    end
  end
  return numbers
end

local function mock_missing_remote_branch(branch)
  t.mock_command("git fetch 'origin' '" .. tostring(branch) .. "'", {
    stdout = "",
    stderr = "fatal: couldn't find remote ref",
    exit_code = 128,
  })
end

local function issue_rest_view_number(rendered)
  return tostring(rendered or ""):match("gh api 'repos/owner/repo/issues/(%d+)'$")
end

return {
  test_liveness_scan_requeues_pr_open_issue_and_observe_replays_reviewing = function()
    local ready_payload = reviewing()
    mock_repo()
    mock_issue_list({ { number = 42, state = "open", updated_at = "2026-06-03T01:02:03Z" } })
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", {
      core.state_marker(proposal_id, "pr-open", version),
      core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", version, "dev"),
    })
    mock_empty_pr_list()

    local scanned = run_liveness_scan("liveness-scan-pr-open")
    t.eq(scanned.exit_code, 0)
    local raised = find_raise(scanned.raises, "github-proxy.github_entity_changed")
    t.is_true(raised ~= nil)
    t.eq(raised.payload.type, "issue")
    t.eq(raised.payload.repo, repo)
    t.eq(raised.payload.number, 42)
    t.eq(raised.payload.updated_at, "2026-06-03T01:02:03Z")
    t.eq(raised.payload.source, "liveness-scan")
    t.eq(raised.payload.source_ref.ref, "owner/repo#issue/42")
    t.is_true(tostring(raised.payload.dedup_key):find("liveness%-scan", 1) ~= nil)

    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", {
      core.state_marker(proposal_id, "pr-open", version),
      core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", version, "dev"),
    })
    mock_linked_pr_state({}, nil, nil, 2)
    local observed = run_observe(issue({
      dedup_key = raised.payload.dedup_key,
      source = raised.payload.source,
      source_ref = raised.payload.source_ref,
    }), opts("liveness-scan-observe-pr-open"))
    t.eq(observed.exit_code, 0)
    local reviewing_raise = find_raise(observed.raises, "devloop_reviewing")
    t.is_true(reviewing_raise ~= nil)
    t.eq(reviewing_raise.payload.proposal_id, ready_payload.proposal_id)
    t.eq(reviewing_raise.payload.pr_number, ready_payload.pr_number)
    t.eq(reviewing_raise.payload.version, version .. "/review-loop/1")
  end,

  test_observe_pr_open_closed_unmerged_pr_redrives_ready = function()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", {
      core.state_marker(proposal_id, "pr-open", version),
      core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", version, "dev"),
    })
    mock_linked_pr_state({}, "CLOSED", nil, 2)

    local result = run_observe(issue({
      dedup_key = "liveness-scan/closed-pr-open",
      source = "liveness-scan",
    }), opts("observe-pr-open-closed-redrive"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    local ready_raise = find_raise(result.raises, "devloop_ready")
    t.is_true(ready_raise ~= nil)
    t.eq(ready_raise.payload.proposal_id, proposal_id)
    t.eq(ready_raise.payload.source_ref.ref, "owner/repo#issue/42")
    t.is_true(tostring(ready_raise.payload.dedup_key):find("reimplement/1", 1, true) ~= nil)
  end,

  test_observe_reviewing_closed_unmerged_pr_redrives_ready = function()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:reviewing" }, "OPEN", {
      core.state_marker(proposal_id, "reviewing", version),
      core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", version, "dev"),
    })
    mock_linked_pr_state({}, "CLOSED", nil, 2)

    local result = run_observe(issue({
      dedup_key = "liveness-scan/closed-reviewing",
      source = "liveness-scan",
    }), opts("observe-reviewing-closed-redrive"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    local ready_raise = find_raise(result.raises, "devloop_ready")
    t.is_true(ready_raise ~= nil)
    t.eq(ready_raise.payload.proposal_id, proposal_id)
    t.eq(ready_raise.payload.source_ref.ref, "owner/repo#issue/42")
    t.is_true(tostring(ready_raise.payload.dedup_key):find("reimplement/1", 1, true) ~= nil)
  end,

  test_observe_reviewing_absent_pr_redrives_ready = function()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:reviewing" }, "OPEN", {
      core.state_marker(proposal_id, "reviewing", version),
      core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", version, "dev"),
    })
    mock_linked_pr_absent(2)

    local result = run_observe(issue({
      dedup_key = "liveness-scan/absent-reviewing",
      source = "liveness-scan",
    }), opts("observe-reviewing-absent-redrive"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    local ready_raise = find_raise(result.raises, "devloop_ready")
    t.is_true(ready_raise ~= nil)
    t.eq(ready_raise.payload.proposal_id, proposal_id)
    t.is_true(tostring(ready_raise.payload.dedup_key):find("reimplement/1", 1, true) ~= nil)
  end,

  test_observe_reviewing_merged_pr_marks_issue_merged_without_reimplementing = function()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:reviewing" }, "OPEN", {
      core.state_marker(proposal_id, "reviewing", version),
      core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", version, "dev"),
    })
    mock_linked_pr_state({}, "MERGED", nil, 2)

    local result = run_observe(issue({
      dedup_key = "liveness-scan/merged-reviewing",
      source = "liveness-scan",
    }), opts("observe-reviewing-merged"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    t.eq(find_raise(result.raises, "devloop_ready"), nil)
    local comment_raise = find_raise(result.raises, "github-proxy.github_issue_comment_request")
    t.is_true(comment_raise ~= nil)
    t.is_true(tostring(comment_raise.payload.body):find('state="merged"', 1, true) ~= nil)
    t.is_true(tostring(comment_raise.payload.body):find("fkst:github-devloop:merged:v1", 1, true) ~= nil)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.is_true(label_raise ~= nil)
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:merged")
  end,

  test_liveness_scan_skips_terminal_issue = function()
    mock_repo()
    mock_issue_list({ { number = 42, state = "open", updated_at = "2026-06-03T01:02:03Z" } })
    mock_issue_state({ "fkst-dev:merged" }, "OPEN", {
      core.state_marker(proposal_id, "merged", version),
    })
    mock_empty_pr_list()

    assert_no_entity_change(run_liveness_scan("liveness-scan-terminal"))
  end,

  test_liveness_scan_skips_issue_without_state = function()
    mock_repo()
    mock_issue_list({ { number = 42, state = "open", updated_at = "2026-06-03T01:02:03Z" } })
    mock_issue_state({ "fkst-dev:enabled" }, "OPEN", {})
    mock_empty_pr_list()

    assert_no_entity_change(run_liveness_scan("liveness-scan-no-state"))
  end,

  test_liveness_scan_requeues_every_non_terminal_issue_marker_state = function()
    local issues = {}
    local expected = {}
    local number = 100
    for _, row in ipairs(core.restart_transition_table()) do
      number = number + 1
      local state = row.from_state
      table.insert(issues, {
        number = number,
        state = "open",
        updated_at = "2026-06-03T01:02:03Z",
      })
      mock_issue_state_number(number, { "fkst-dev:enabled", core.state_label(state) }, "OPEN", {
        core.state_marker(core.proposal_id(repo, number), state, version),
      })
      if row.terminal == false then
        expected[number] = state
      end
    end
    mock_repo()
    mock_issue_list(issues)
    mock_empty_pr_list()

    local result = run_liveness_scan("liveness-scan-non-terminal-issue-marker-conformance")
    t.eq(result.exit_code, 0)
    local raised = entity_change_issue_numbers(result)
    for issue_number, state in pairs(expected) do
      t.eq(raised[issue_number], true, "non-terminal issue marker state not sweep-reachable: " .. tostring(state))
    end
    for _, row in ipairs(core.restart_transition_table()) do
      if row.terminal == true then
        for _, item in ipairs(issues) do
          if item.number ~= nil
            and core.current_state({
              { body = core.state_marker(core.proposal_id(repo, item.number), row.from_state, version), author_login = "fkst-test-bot" },
            }, core.proposal_id(repo, item.number)).state == row.from_state then
            t.eq(raised[item.number], nil, "terminal issue marker state was requeued: " .. tostring(row.from_state))
          end
        end
      end
    end
  end,

  test_liveness_scan_requeues_ready_dependency_hold = function()
    mock_repo()
    mock_issue_list({ { number = 42, state = "open", updated_at = "2026-06-03T01:02:03Z" } })
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:ready", "fkst-dev:blocked-on-dependency" }, "OPEN", {
      core.state_marker(proposal_id, "ready", version),
      core.dependency_wait_marker(proposal_id, version, { 7 }),
    })
    mock_empty_pr_list()

    local result = run_liveness_scan("liveness-scan-ready-dependency-hold")
    t.eq(result.exit_code, 0)
    local raised = find_raise(result.raises, "github-proxy.github_entity_changed")
    t.is_true(raised ~= nil)
    t.eq(raised.payload.type, "issue")
    t.eq(raised.payload.source, "liveness-scan")
    t.is_true(tostring(raised.payload.dedup_key):find("liveness%-scan", 1) ~= nil)
  end,

  test_liveness_scan_reinjected_implementing_round_trips_with_frozen_version = function()
    local event = ready()
    local branch = deterministic_branch_for(event)
    local stuck = {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
      core.implement_attempt_marker(event.proposal_id, event.dedup_key, 1, tostring(now() - 7201)),
    }
    mock_repo()
    mock_issue_list({ { number = 42, state = "open", updated_at = "2026-06-03T01:02:03Z" } })
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:implementing" }, "OPEN", stuck)
    mock_empty_pr_list()

    local scanned = run_liveness_scan("liveness-scan-implementing-redrive-scan")
    t.eq(scanned.exit_code, 0)
    local entity = find_raise(scanned.raises, "github-proxy.github_entity_changed")
    t.eq(entity ~= nil, true)
    t.eq(entity.payload.source, "liveness-scan")
    t.is_true(tostring(entity.payload.dedup_key):find("liveness%-scan", 1) ~= nil)

    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:implementing" }, "OPEN", stuck)
    local observed = run_observe(issue({
      dedup_key = entity.payload.dedup_key,
      source = entity.payload.source,
      source_ref = entity.payload.source_ref,
    }), opts("liveness-scan-implementing-redrive-observe"))
    t.eq(observed.exit_code, 0)
    local reraised = find_raise(observed.raises, "devloop_ready")
    t.eq(reraised ~= nil, true)
    t.eq(reraised.payload.proposal_id, event.proposal_id)
    t.eq(reraised.payload.dedup_key, event.dedup_key)
    t.eq(core.implementation_attempt_version(reraised.payload.dedup_key, reraised.payload.impl_retry_attempt), event.dedup_key)

    mock_issue_implement({ "fkst-dev:implementing" }, {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
      core.implement_attempt_marker(event.proposal_id, event.dedup_key, 1, "1"),
    })
    mock_missing_remote_branch(branch)
    t.mock_command("show-ref --verify --quiet", { stdout = "", stderr = "", exit_code = 1 })
    mock_fresh_implement_worktree()
    mock_implement_codex(0, "implemented after liveness scan re-drive")
    mock_git_status(" M packages/github-devloop/core.lua\n")
    mock_git_commit("def456", branch)
    mock_issue_implement({ "fkst-dev:implementing" }, {
      core.state_marker(event.proposal_id, "implementing", event.dedup_key),
      core.implement_attempt_marker(event.proposal_id, event.dedup_key, 1, "1"),
    })

    local implemented = run_implement(reraised.payload, opts("liveness-scan-implementing-redrive-implement"))
    t.eq(implemented.exit_code, 0)
    t.eq(count_calls("codex exec"), 1)
    t.eq(find_raise(implemented.raises, "devloop_open_pr") ~= nil, true)
  end,

  test_liveness_scan_requeues_open_pr_with_non_terminal_state = function()
    local event = reviewing()
    mock_repo()
    mock_issue_list({})
    mock_pr_list({ { number = 7, state = "open", updated_at = "2026-06-04T01:02:03Z" } })
    mock_pr_state({
      core.pr_origin_marker(event.proposal_id, 42, "devloop-owner-repo-42-01HY", event.version, "dev"),
      core.state_marker(event.proposal_id, "reviewing", event.version),
    }, "OPEN")
    t.mock_command(core.gh_issue_view_claim_cmd(repo, 42), {
      stdout = '{"assignees":[{"login":"fkst-test-bot"}],"author":{"login":"fkst-test-bot"}}\n',
      stderr = "",
      exit_code = 0,
    })

    local result = run_liveness_scan("liveness-scan-pr-reviewing")
    t.eq(result.exit_code, 0)
    local raised = find_raise(result.raises, "github-proxy.github_entity_changed")
    t.is_true(raised ~= nil)
    t.eq(raised.payload.type, "pr")
    t.eq(raised.payload.repo, repo)
    t.eq(raised.payload.number, 7)
    t.eq(raised.payload.source_ref.ref, "owner/repo#pr/7")
    t.is_true(tostring(raised.payload.dedup_key):find("liveness%-scan", 1) ~= nil)

    mock_pr_state({
      core.pr_origin_marker(event.proposal_id, 42, "devloop-owner-repo-42-01HY", event.version, "dev"),
      core.state_marker(event.proposal_id, "reviewing", event.version),
    }, "OPEN")
    local observed = run_observe_pr({
      schema = "github-proxy.v1",
      type = "pr",
      repo = repo,
      number = 7,
      updated_at = raised.payload.updated_at,
      dedup_key = raised.payload.dedup_key,
      source = raised.payload.source,
      source_ref = raised.payload.source_ref,
    }, opts("liveness-scan-observe-pr-reviewing"))
    t.eq(observed.exit_code, 0)
    local reviewing_raise = find_raise(observed.raises, "devloop_reviewing")
    t.is_true(reviewing_raise ~= nil)
    t.eq(reviewing_raise.payload.version, event.version .. "/review-loop/1")
  end,

  test_liveness_scan_skips_other_owned_pr_before_reinjecting = function()
    local event = reviewing()
    mock_repo()
    mock_issue_list({})
    mock_pr_list({ { number = 7, state = "open", updated_at = "2026-06-04T01:02:03Z" } })
    mock_pr_state({
      core.pr_origin_marker(event.proposal_id, 42, "devloop-owner-repo-42-01HY", event.version, "dev"),
      core.state_marker(event.proposal_id, "reviewing", event.version),
    }, "OPEN")
    t.mock_command(core.gh_issue_view_claim_cmd(repo, 42), {
      stdout = '{"assignees":[{"login":"human"}],"author":{"login":"fkst-test-bot"}}\n',
      stderr = "",
      exit_code = 0,
    })

    assert_no_entity_change(run_liveness_scan("liveness-scan-pr-other-owned"))
  end,

  test_liveness_scan_caps_before_fresh_entity_views = function()
    local items = {}
    for number = 1, 101 do
      table.insert(items, { number = number, state = "open", updated_at = "2026-06-03T01:02:03Z" })
    end
    mock_repo()
    mock_issue_list(items)
    mock_empty_pr_list()
    for number = 1, 101 do
      mock_issue_state_number(number, { "fkst-dev:enabled", "fkst-dev:merged" }, "OPEN", {
        core.state_marker(core.proposal_id(repo, number), "merged", "v-" .. tostring(number)),
      })
    end

    local result = run_liveness_scan("liveness-scan-cap-before-views")
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_entity_changed"), nil)
    local views = 0
    for _, call in ipairs(t.command_calls()) do
      if issue_rest_view_number(call.rendered) ~= nil then
        views = views + 1
      end
    end
    t.eq(views, 100)
  end,

  test_liveness_scan_uses_cursor_first_batch_on_large_board = function()
    local items = {}
    for number = 1, 101 do
      table.insert(items, { number = number, state = "open", updated_at = "2026-06-03T01:02:03Z" })
    end
    mock_repo()
    mock_issue_list(items)
    mock_empty_pr_list()
    for number = 1, 101 do
      mock_issue_state_number(number, { "fkst-dev:enabled", "fkst-dev:merged" }, "OPEN", {
        core.state_marker(core.proposal_id(repo, number), "merged", "v-" .. tostring(number)),
      })
    end

    local tick = "2026-06-03T01:32:04Z"
    local result = run_liveness_scan_at("liveness-scan-rotates-large-board", tick)
    t.eq(result.exit_code, 0)

    local viewed = {}
    for _, call in ipairs(t.command_calls()) do
      local issue_number = issue_rest_view_number(call.rendered)
      if issue_number ~= nil then
        viewed[tonumber(issue_number)] = true
      end
    end
    t.eq(viewed[1], true)
    t.eq(viewed[100], true)
    t.eq(viewed[101], nil)
  end,

  test_liveness_scan_cursor_covers_large_board_across_k_ticks = function()
    local items = {}
    for number = 1, 250 do
      table.insert(items, { number = number, state = "open", updated_at = "2026-06-03T01:02:03Z" })
    end

    local viewed = {}
    local run_opts = opts("liveness-scan-cursor-k")
    for tick = 1, 3 do
      mock_repo()
      mock_issue_list(items)
      mock_empty_pr_list()
      for number = 1, 250 do
        mock_issue_state_number(number, { "fkst-dev:enabled", "fkst-dev:merged" }, "OPEN", {
          core.state_marker(core.proposal_id(repo, number), "merged", "v-" .. tostring(number)),
        })
      end

      local result = run_liveness_scan_at("liveness-scan-cursor-k", tostring(tick), run_opts)
      t.eq(result.exit_code, 0)

      for _, call in ipairs(t.command_calls()) do
        local issue_number = issue_rest_view_number(call.rendered)
        if issue_number ~= nil then
          viewed[tonumber(issue_number)] = true
        end
      end
    end

    for number = 1, 250 do
      t.eq(viewed[number], true)
    end
  end,

  test_liveness_scan_defers_slow_issue_view_without_retry_failure = function()
    mock_repo()
    mock_issue_list({ { number = 42, state = "open", updated_at = "2026-06-03T01:02:03Z" } })
    mock_empty_pr_list()
    t.mock_command("gh api 'repos/owner/repo/issues/42'", {
      stdout = "",
      stderr = "timed out",
      exit_code = 124,
    })

    local result = run_liveness_scan("liveness-scan-view-timeout-deferred")
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "github-proxy.github_entity_changed"), nil)
  end,
}
