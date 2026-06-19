local t = fkst.test
local core = require("core")

local function package_root()
  local source = package.searchpath("tests.unsupported_payload_test", package.path)
  return source:match("(.+)/tests/unsupported_payload_test%.lua$")
end

local function department_paths()
  local root = package_root()
  local result = {}
  local find = assert(io.popen("find " .. root .. "/departments -mindepth 2 -maxdepth 2 -name main.lua | sort"))
  for path in find:lines() do
    local rel = path:sub(#root + 2)
    table.insert(result, rel)
  end
  local ok = find:close()
  if ok == false then
    error("github-devloop: department discovery failed")
  end
  return result
end

local function read_file(path)
  local handle = assert(io.open(path, "r"))
  local body = handle:read("*a")
  handle:close()
  return body
end

local function department_source(path)
  return read_file(package_root() .. "/" .. path)
end

local function load_department_spec(path)
  local old_pipeline = pipeline
  local module = dofile(package_root() .. "/" .. path)
  pipeline = old_pipeline
  if type(module) ~= "table" or type(module.spec) ~= "table" then
    error("github-devloop: department spec missing for " .. tostring(path))
  end
  return module.spec
end

local function production_queue_name(queue)
  if tostring(queue):find("%.", 1, false) ~= nil then
    return queue
  end
  return "github-devloop." .. tostring(queue)
end

local function issue_consensus_payload()
  return {
    schema = "consensus.consensus_reached.v1",
    proposal_id = "github-devloop/issue/owner/repo/42",
    decision = "approve",
    body = "All angles approve.",
    dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    source_ref = { kind = "external", ref = "owner/repo#issue/42" },
  }
end

local function review_proposal_id()
  return core.pr_review_proposal_id("owner/repo", 7, "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z", "def456")
end

local function review_consensus_payload()
  local proposal_id = review_proposal_id()
  return {
    schema = "consensus.consensus_reached.v1",
    proposal_id = proposal_id,
    decision = "approve",
    body = "All angles approve.",
    dedup_key = "consensus:" .. proposal_id .. "/review",
    source_ref = { kind = "external", ref = "owner/repo#pr/7" },
  }
end

local function issue_unresolved_payload()
  return {
    schema = "consensus.consensus_converge.v1",
    proposal_id = "github-devloop/issue/owner/repo/42",
    dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    source_ref = { kind = "external", ref = "owner/repo#issue/42" },
  }
end

local function review_unresolved_payload()
  local proposal_id = review_proposal_id()
  return {
    schema = "consensus.consensus_converge.v1",
    proposal_id = proposal_id,
    dedup_key = "consensus:" .. proposal_id .. "/review",
    source_ref = { kind = "external", ref = "owner/repo#pr/7" },
  }
end

local function issue_entity_payload()
  return {
    schema = "github-proxy.v1",
    type = "issue",
    repo = "owner/repo",
    number = 42,
    title = "Implement decision recorder",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    labels = { "fkst-dev:enabled" },
    dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
    source_ref = { kind = "external", ref = "owner/repo#issue/42" },
  }
end

local function pr_entity_payload()
  return {
    schema = "github-proxy.v1",
    type = "pr",
    repo = "owner/repo",
    number = 7,
    title = "Implement decision recorder",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    dedup_key = "owner/repo#pr#7@2026-06-03T01:02:03Z",
    source_ref = { kind = "external", ref = "owner/repo#pr/7" },
  }
end

local function run_department_with_logs(path, event)
  local result = t.run_department(path, event)
  t.is_true(type(result) == "table")

  local captured = {}
  local old_log = log

  log = {
    info = function(message)
      table.insert(captured, tostring(message))
    end,
    warn = function(message)
      table.insert(captured, tostring(message))
    end,
    error = function(message)
      table.insert(captured, tostring(message))
    end,
  }

  local old_pipeline = pipeline
  local ok, err = pcall(function()
    dofile(package_root() .. "/" .. path)
    pipeline(event)
  end)
  pipeline = old_pipeline
  log = old_log
  return ok, tostring(err or ""), table.concat({
    tostring(result.error or ""),
    table.concat(captured, "\n"),
  }, "\n")
end

local function branch_tick_payload()
  return { schema = "github-devloop.branch-tick.v1" }
end

local function payload_for_queue(queue)
  local payloads = {
    ["consensus.consensus_converge"] = issue_unresolved_payload(),
    ["consensus.consensus_reached"] = issue_consensus_payload(),
    dead_letter = {
      delivery_id = "delivery/v1/raised/queue/github-devloop.devloop_ready/dept/github-devloop.implement/01HY",
      queue = "github-devloop.devloop_ready",
      dept = "github-devloop.implement",
      dedup_key = "dead-letter-test",
      attempt = 1,
      error = "test error",
      source_ref = { kind = "external", ref = "owner/repo#issue/42" },
    },
    devloop_branch_tick = branch_tick_payload(),
    devloop_decompose = core.build_devloop_decompose_payload(core.build_devloop_fix_reconcile_payload({
      proposal_id = "github-devloop/issue/owner/repo/42",
      review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/3", "def456"),
      review_dedup_key = "consensus:" .. core.pr_review_proposal_id("owner/repo", 7, "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/3", "def456") .. "/review",
      reviewed_head_sha = "def456",
      pr_number = 7,
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }, "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/3")),
    devloop_doctor_tick = { schema = "github-devloop.doctor-tick.v1" },
    devloop_ensure_repo_tick = { schema = "github-devloop.ensure-repo-tick.v1" },
    devloop_fix_reconcile = core.build_devloop_fix_reconcile_payload({
      proposal_id = "github-devloop/issue/owner/repo/42",
      review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/3", "def456"),
      review_dedup_key = "consensus:" .. core.pr_review_proposal_id("owner/repo", 7, "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/3", "def456") .. "/review",
      reviewed_head_sha = "def456",
      pr_number = 7,
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }, "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/3"),
    devloop_fixing = {
      schema = "github-devloop.fixing.v1",
      proposal_id = "github-devloop/issue/owner/repo/42",
      pr_number = 7,
      version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/1",
      review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z", "def456"),
      review_dedup_key = "consensus:" .. core.pr_review_proposal_id("owner/repo", 7, "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z", "def456") .. "/review",
      reviewed_head_sha = "def456",
      blocking_gap = "missing regression guard",
      dedup_key = "fixing/github-devloop/issue/owner/repo/42/v1",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    },
    devloop_intake_candidate = core.build_devloop_intake_candidate_payload("owner/repo", "42", "2026-06-03T01:02:03Z"),
    devloop_intake_probe_tick = { schema = "github-devloop.intake-probe-tick.v1" },
    devloop_intake_tick = { schema = "github-devloop.intake-tick.v1" },
    devloop_liveness_tick = { schema = "github-devloop.tick.v1" },
    devloop_merge_queue_tick = core.merge_queue_tick_payload("owner/repo", 6, {
      proposal_id = "github-devloop/issue/owner/repo/42",
      pr_number = 7,
      version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
      review_proposal_id = review_proposal_id(),
      review_dedup_key = "consensus:" .. review_proposal_id() .. "/review",
      reviewed_head_sha = "def456",
      head_sha = "def456",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }),
    devloop_merge_ready = core.build_devloop_merge_ready_payload("github-devloop/issue/owner/repo/42", 7, "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z", {
      review_proposal_id = core.pr_review_proposal_id("owner/repo", 7, "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z", "def456"),
      review_dedup_key = "consensus:" .. core.pr_review_proposal_id("owner/repo", 7, "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z", "def456") .. "/review",
      reviewed_head_sha = "def456",
    }, { kind = "external", ref = "owner/repo#pr/7" }),
    devloop_observe_tick = { schema = "github-devloop.observe-tick.v1" },
    devloop_open_pr = core.build_devloop_open_pr_payload("owner/repo", "42", {
      proposal_id = "github-devloop/issue/owner/repo/42",
      dedup_key = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
      source_ref = { kind = "external", ref = "owner/repo#issue/42" },
    }, "devloop-owner-repo-42-01HY", "def456", "dev"),
    devloop_ready = {
      schema = "github-devloop.ready.v1",
      proposal_id = "github-devloop/issue/owner/repo/42",
      dedup_key = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
      source_ref = { kind = "external", ref = "owner/repo#issue/42" },
    },
    devloop_ready_session = core.build_devloop_ready_payload({
      proposal_id = "github-devloop/issue/owner/repo/42",
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
      source_ref = { kind = "external", ref = "owner/repo#issue/42" },
      include_ready_hand_off = true,
    }),
    devloop_reconcile = core.build_devloop_reconcile_payload({
      schema = "consensus.consensus_converge.v1",
      proposal_id = "github-devloop/issue/owner/repo/42",
      dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/loop/3",
      source_ref = { kind = "external", ref = "owner/repo#issue/42" },
    }, 3, "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"),
    devloop_review_meta = core.build_devloop_review_meta_payload({
      schema = "consensus.consensus_converge.v1",
      proposal_id = review_proposal_id(),
      dedup_key = "consensus:" .. review_proposal_id() .. "/review/loop/2",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }, "github-devloop/issue/owner/repo/42", "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z", 7, 3),
    devloop_review_reconcile = core.build_devloop_review_reconcile_payload({
      schema = "consensus.consensus_converge.v1",
      proposal_id = review_proposal_id(),
      dedup_key = "consensus:" .. review_proposal_id() .. "/review/loop/3",
      round = 3,
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    }, 3, "github-devloop/issue/owner/repo/42", "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z", "def456"),
    devloop_reviewing = {
      schema = "github-devloop.reviewing.v1",
      proposal_id = "github-devloop/issue/owner/repo/42",
      pr_number = 7,
      version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
      dedup_key = "reviewing/github-devloop/issue/owner/repo/42/ready-consensus-github-devloop-issue-owner-repo-42-2026-06-03T01-02-03Z/7",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    },
    devloop_rollup_ready = core.rollup_ready_payload("owner/repo", "dev", "integration/dev", 7, "def456"),
    devloop_substrate_ref_tick = { schema = "github-devloop.substrate-ref-tick.v1" },
    board_digest_probe = {
      mode = "block",
      repo = "owner/repo",
      tick = "2026-06-10T02:12:03Z",
    },
    cache_seed = {
      key = "github-devloop/test-cache-key",
      value = "test-cache-value",
    },
    context_bundle_probe = {
      mode = "round_trip",
      root = "/tmp/fkst-packages-test/github-devloop-unsupported-context-bundle",
    },
    devloop_sync_conflict = {
      schema = "github-devloop.v1",
      repo = "owner/repo",
      upstream_branch = "dev",
      integration_branch = "integration/dev",
      upstream_sha = "abc123",
      integration_sha = "def456",
      dedup_key = core.branch_sync_dedup_key("owner/repo", "dev", "integration/dev", "abc123"),
      source_ref = core.branch_sync_source_ref("owner/repo", "dev", "integration/dev"),
    },
    devloop_timeout_reconcile = core.build_devloop_timeout_reconcile_payload({
      from_state = "ready",
    }, {
      version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/timeout/ready/1",
    }, "github-devloop/issue/owner/repo/42", { kind = "external", ref = "owner/repo#issue/42" }, 1),
    ["github-proxy.github_comment_written"] = {
      schema = "github-proxy.comment-written.v1",
      repo = "owner/repo",
      target = "issue",
      issue_number = 42,
      comment_id = "123456",
      request_dedup_key = "github-devloop/issue/owner/repo/42/comment/approve/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
      dedup_key = "github-devloop/issue/owner/repo/42/comment/approve/written/123456",
      source_ref = { kind = "external", ref = "owner/repo#issue/42" },
      handoff = {
        kind = "github-devloop.ready",
        proposal_id = "github-devloop/issue/owner/repo/42",
        version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
        marker_version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
        source_ref = { kind = "external", ref = "owner/repo#issue/42" },
      },
    },
    ["github-proxy.github_entity_changed"] = issue_entity_payload(),
    ["github-proxy.github_pr_opened"] = {
      schema = "github-proxy.pr-opened.v1",
      repo = "owner/repo",
      issue_number = 42,
      proposal_id = "github-devloop/issue/owner/repo/42",
      impl_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
      pr_number = 7,
      branch = "devloop-owner-repo-42-01HY",
      head_sha = "def456",
      base_branch = "dev",
      dedup_key = "github-pr-opened/owner/repo/7",
      source_ref = { kind = "external", ref = "owner/repo#pr/7" },
    },
  }
  local payload = payloads[queue]
  if payload == nil then
    error("github-devloop: no production-shaped queue fixture for " .. tostring(queue))
  end
  return payload
end

local function payload_for_department_queue(path, queue)
  if path == "departments/review_result/main.lua" and queue == "consensus.consensus_reached" then
    return review_consensus_payload()
  end
  if path == "departments/review_loop/main.lua" and queue == "consensus.consensus_converge" then
    return review_unresolved_payload()
  end
  if path == "departments/observe_pr/main.lua" and queue == "github-proxy.github_entity_changed" then
    return pr_entity_payload()
  end
  return payload_for_queue(queue)
end

local function assert_no_unsupported_queue_fallthrough(path, queue, ok, err, logs)
  local text = tostring(err or "") .. "\n" .. tostring(logs or "")
  if text:find("consumed-queue-unrouted", 1, true) ~= nil then
    error("github-devloop: consumed queue is unrouted for " .. path .. " queue=" .. queue .. ": " .. text)
  end
  if text:find("unsupported event payload", 1, true) ~= nil
    or text:find("unsupported sync conflict payload", 1, true) ~= nil
    or text:find("skip-foreign(payload)", 1, true) ~= nil
    or text:find("skip-foreign(pr)", 1, true) ~= nil
    or text:find("skip-foreign(proposal_id)", 1, true) ~= nil
    or text:find("skip-foreign(source_ref)", 1, true) ~= nil then
    error("github-devloop: production-shaped consumed queue fell through unsupported path for " .. path .. " queue=" .. queue .. ": " .. text)
  end
end

local cases = {
  {
    dept = "loop",
    path = "departments/loop/main.lua",
    queue = "consensus.consensus_converge",
  },
  {
    dept = "review_loop",
    path = "departments/review_loop/main.lua",
    queue = "consensus.consensus_converge",
  },
  {
    dept = "review_result",
    path = "departments/review_result/main.lua",
    queue = "consensus.consensus_reached",
  },
  {
    dept = "review_meta",
    path = "departments/review_meta/main.lua",
    queue = "devloop_review_meta",
  },
  {
    dept = "decompose",
    path = "departments/decompose/main.lua",
    queue = "devloop_decompose",
  },
  {
    dept = "review_pr",
    path = "departments/review_pr/main.lua",
    queue = "devloop_reviewing",
  },
  {
    dept = "implement",
    path = "departments/implement/main.lua",
    queue = "devloop_ready",
  },
  {
    dept = "consensus_result",
    path = "departments/consensus_result/main.lua",
    queue = "consensus.consensus_reached",
  },
  {
    dept = "open_pr",
    path = "departments/open_pr/main.lua",
    queue = "devloop_open_pr",
  },
  {
    dept = "fix",
    path = "departments/fix/main.lua",
    queue = "devloop_fixing",
  },
  {
    dept = "observe_pr",
    path = "departments/observe_pr/main.lua",
    queue = "github-proxy.github_entity_changed",
  },
  {
    dept = "intake_judge",
    path = "departments/intake_judge/main.lua",
    queue = "devloop_intake_candidate",
  },
  {
    dept = "merge",
    path = "departments/merge/main.lua",
    queue = "devloop_merge_ready",
  },
  {
    dept = "observe_issue",
    path = "departments/observe_issue/main.lua",
    queue = "github-proxy.github_entity_changed",
  },
  {
    dept = "sync_conflict",
    path = "departments/sync_conflict/main.lua",
    queue = "devloop_sync_conflict",
  },
  {
    dept = "reconcile",
    path = "departments/reconcile/main.lua",
    queue = "devloop_reconcile",
  },
  {
    dept = "review_reconcile",
    path = "departments/reconcile/main.lua",
    queue = "devloop_review_reconcile",
  },
  {
    dept = "fix_reconcile",
    path = "departments/reconcile/main.lua",
    queue = "devloop_fix_reconcile",
  },
  {
    dept = "timeout_reconcile",
    path = "departments/reconcile/main.lua",
    queue = "devloop_timeout_reconcile",
  },
  {
    dept = "rollup_merge",
    path = "departments/rollup_merge/main.lua",
    queue = "devloop_rollup_ready",
  },
}

return {
  test_consumed_queue_dispatch_accepts_namespaced_declared_queues = function()
    local routed = {}
    local spec = {
      consumes = { "devloop_ready", "devloop_ready_session" },
    }
    local handled = core.dispatch_consumed_queue("test", spec, {
      queue = "github-devloop.devloop_ready_session",
      payload = {},
    }, {
      devloop_ready = function()
        table.insert(routed, "ready")
      end,
      devloop_ready_session = function()
        table.insert(routed, "ready-session")
      end,
    })

    t.eq(handled, true)
    t.eq(routed[1], "ready-session")
  end,

  test_consumed_queue_dispatch_fail_closed_when_declared_queue_is_unrouted = function()
    t.raises(function()
      core.dispatch_consumed_queue("test", {
        consumes = { "devloop_ready", "devloop_ready_session" },
      }, {
        queue = "github-devloop.devloop_ready_session",
        payload = {},
      }, {
        devloop_ready = function() end,
      })
    end)
  end,

  test_consumed_queue_dispatch_skips_foreign_queue_without_error = function()
    local handled = core.dispatch_consumed_queue("test", {
      consumes = { "devloop_ready" },
    }, {
      queue = "github-proxy.github_entity_changed",
      payload = {},
    }, {
      devloop_ready = function()
        error("github-devloop: unexpected foreign dispatch")
      end,
    })

    t.eq(handled, false)
  end,

  test_event_queue_matches_namespaced_session_queue = function()
    t.eq(core.event_queue_matches({ queue = "github-devloop.devloop_ready_session" }, "devloop_ready_session"), true)
    t.eq(core.event_queue_matches({ queue = "devloop_ready_session" }, "devloop_ready_session"), true)
    t.eq(core.event_queue_matches({ queue = "github-devloop.devloop_ready" }, "devloop_ready_session"), false)
  end,

  test_all_departments_accept_production_namespaced_consumed_queues = function()
    for _, path in ipairs(department_paths()) do
      local spec = load_department_spec(path)
      for _, queue in ipairs(spec.consumes or {}) do
        local event = {
          queue = production_queue_name(queue),
          payload = payload_for_department_queue(path, queue),
        }
        local ok, err, logs = run_department_with_logs(path, event)
        assert_no_unsupported_queue_fallthrough(path, queue, ok, err, logs)
      end
    end
  end,

  test_observers_fail_closed_for_declared_but_unrouted_queue = function()
    for _, path in ipairs({
      "departments/observe_issue/main.lua",
      "departments/observe_pr/main.lua",
    }) do
      local old_pipeline = pipeline
      local module = dofile(package_root() .. "/" .. path)
      table.insert(module.spec.consumes, "devloop_unrouted_probe")

      local ok, err = pcall(function()
        pipeline({
          queue = "github-devloop.devloop_unrouted_probe",
          payload = {},
        })
      end)
      pipeline = old_pipeline

      t.eq(ok, false)
      t.is_true(tostring(err):find("consumed%-queue%-unrouted") ~= nil)
    end
  end,

  test_unsupported_payload_consumers_skip_non_table_payloads = function()
    for _, case in ipairs(cases) do
      for _, payload in ipairs({ false, "foreign-payload", 42 }) do
        local result = t.run_department(case.path, {
          queue = case.queue,
          payload = payload,
        })

        t.eq(result.exit_code, 0)
        t.eq(#result.raises, 0)
      end
    end
  end,

  test_payload_field_returns_nil_for_userdata = function()
    local userdata_payload = assert(io.tmpfile())
    t.eq(core.payload_field(userdata_payload, "dedup_key"), nil)
    userdata_payload:close()
  end,
}
