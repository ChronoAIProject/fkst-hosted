local base_ids = require("devloop.base_ids")
local context_bundle = require("devloop.context_bundle")
local devloop_base = require("devloop.base")
local entity_lib = require("devloop.entity")
local graph = require("testkit.graph")
local h = require("tests.devloop_helpers")
local m_builders = require("devloop.markers.builders")
local payloads_builders = require("devloop.payloads.builders")
local entity_mocks = require("tests.entity_read_mock_helpers")

local t = h.t
local core = h.core

local lifecycle_repo = "owner/lifecycle"
local implementation_repo = "owner/implementation"
local implementation_branch = "fkst-hosted"
local implementation_root = "/runtime/implementation"
local issue_number = 42
local pr_number = 7
local proposal_id = base_ids.proposal_id(lifecycle_repo, issue_number)
local root_version = "ready/consensus-github-devloop/issue/owner/lifecycle/42/2026-07-27T01-02-03Z"
local branch = "devloop-owner-lifecycle-42-01HY"
local old_head = "1111111111111111111111111111111111111111"
local new_head = "2222222222222222222222222222222222222222"
local merge_commit = "3333333333333333333333333333333333333333"
local source_ref = entity_lib.pr_source_ref(implementation_repo, pr_number)
local verdict_label = "⟦FKST:VERDICT⟧"
local reply_label = "⟦FKST:REPLY⟧"

local function delivery_grant_json()
  return '[{"lifecycle_repo":"' .. lifecycle_repo
    .. '","lifecycle_issue":' .. tostring(issue_number)
    .. ',"implementation_repo":"' .. implementation_repo
    .. '","implementation_branch":"' .. implementation_branch
    .. '","implementation_root":"' .. implementation_root .. '"}]'
end

local function origin_marker()
  return m_builders.pr_origin_marker(
    proposal_id,
    issue_number,
    branch,
    root_version,
    implementation_branch,
    implementation_repo
  )
end

local function trusted(body, created_at)
  return {
    body = body,
    author_login = core._test_bot_login,
    created_at = created_at or "2026-07-27T01:03:00Z",
  }
end

local function lifecycle_comments()
  return {
    trusted(core.state_marker(proposal_id, "awaiting-pr", root_version), "2026-07-27T01:03:00Z"),
    trusted(m_builders.pr_link_marker(
      proposal_id,
      pr_number,
      branch,
      root_version,
      implementation_branch,
      implementation_repo
    ), "2026-07-27T01:03:01Z"),
    trusted(m_builders.pr_delegation_marker(
      proposal_id,
      entity_lib.pr_proposal_id(implementation_repo, pr_number),
      pr_number,
      root_version,
      "g1",
      implementation_repo
    ), "2026-07-27T01:03:02Z"),
  }
end

local function mock_command_repeated(command, result, times)
  for _ = 1, times or 1 do
    t.mock_command(command, {
      stdout = result.stdout or "",
      stderr = result.stderr or "",
      exit_code = result.exit_code or 0,
    })
  end
end

local function mock_write_modes(values)
  local command = devloop_base.read_env_command("FKST_GITHUB_WRITE")
  local calls_before = h.count_calls(command)
  for _, value in ipairs(values) do
    t.mock_command(command, {
      stdout = value,
      stderr = "",
      exit_code = 0,
    })
  end
  return function()
    local calls_after = h.count_calls(command)
    local expected_after = calls_before + #values
    if calls_after ~= expected_after then
      error(
        "write-mode read mismatch: before=" .. tostring(calls_before)
          .. " expected_after=" .. tostring(expected_after)
          .. " actual_after=" .. tostring(calls_after),
        2
      )
    end
  end
end

local function mock_cross_repo_environment()
  mock_command_repeated(devloop_base.read_env_command("FKST_DEVLOOP_DELIVERY_GRANTS"), {
    stdout = delivery_grant_json(),
  }, 80)
  mock_command_repeated(devloop_base.read_env_command("FKST_GITHUB_BOT_LOGIN"), {
    stdout = core._test_bot_login,
  }, 80)
  mock_command_repeated(devloop_base.read_env_command("FKST_DEVLOOP_UPSTREAM_BRANCH"), {
    stdout = "dev",
  }, 80)
  mock_command_repeated(devloop_base.read_env_command("FKST_DEVLOOP_INTEGRATION_BRANCH"), {
    stdout = "",
  }, 80)
  mock_command_repeated(devloop_base.read_runtime_root_cmd(), {
    stdout = "/tmp/fkst-packages-test/github-devloop-integration/cross-repo-runtime",
  }, 20)
  mock_command_repeated("git -C '" .. implementation_root .. "' rev-parse --show-toplevel", {
    stdout = implementation_root .. "\n",
  }, 40)
  mock_command_repeated("git -C '" .. implementation_root .. "' remote get-url origin", {
    stdout = "https://github.com/" .. implementation_repo .. ".git\n",
  }, 40)
  mock_command_repeated("git -C '" .. implementation_root .. "' rev-parse --abbrev-ref HEAD", {
    stdout = implementation_branch .. "\n",
  }, 40)
  mock_command_repeated("[ -d ", { exit_code = 1 }, 10)
  entity_mocks.mock_issue_view_selector(t, {
    repo = lifecycle_repo,
    number = issue_number,
    assignees = { core._test_bot_login },
    author_login = core._test_bot_login,
  }, "assignees,author", 30)
end

local function mock_pr_origin(comments, head_sha, state, times)
  local fields = {
    repo = implementation_repo,
    number = pr_number,
    head = branch,
    head_sha = head_sha,
    base_branch = implementation_branch,
    base_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    head_repo = implementation_repo,
    state = state or "OPEN",
    comments = comments,
    labels = {},
    merge_commit_sha = state == "MERGED" and merge_commit or nil,
    merged_at = state == "MERGED" and "2026-07-27T02:03:04Z" or nil,
    times = times or 1,
  }
  entity_mocks.mock_pr_read_forms(t, fields)
  entity_mocks.mock_pr_view_selector(t, fields, entity_mocks.pr_origin_selector, times or 1)
end

local function mock_pr_selector(comments, head_sha, selector, fields)
  local selected = fields or {}
  selected.repo = implementation_repo
  selected.number = pr_number
  selected.head = branch
  selected.head_sha = head_sha
  selected.base_branch = implementation_branch
  selected.base_sha = selected.base_sha or "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  selected.head_repo = implementation_repo
  selected.state = selected.state or "OPEN"
  selected.comments = comments
  selected.labels = selected.labels or {}
  if selector == entity_mocks.pr_origin_selector then
    entity_mocks.mock_pr_view_raw_selector(t, selected, selector, {
      stdout = entity_mocks.pr_view_stdout(selected),
      stderr = "",
      exit_code = 0,
    }, 1)
  else
    entity_mocks.mock_pr_view_selector(t, selected, selector, 1)
  end
end

local function run_to_consumer(queue, payload, consumer, max_steps)
  local ok, trace = pcall(function()
    return graph.require_quiescent(graph.run({
      queue = queue,
      payload = payload,
      source_ref = {
        kind = "external",
        reference = source_ref.ref,
      },
    }, { max_steps = max_steps or 16 }))
  end)
  if not ok then
    error(
      "cross-repo graph stage failed queue=" .. tostring(queue)
        .. " consumer=" .. tostring(consumer)
        .. ": " .. tostring(trace),
      2
    )
  end
  local step = graph.require_delivery(trace, {
    queue = queue,
    consumer = consumer,
  })
  if step.status ~= "accepted" or step.exit_code ~= 0 then
    error(
      "cross-repo graph delivery failed queue=" .. tostring(queue)
        .. " consumer=" .. tostring(consumer)
        .. " exit_code=" .. tostring(step.exit_code)
        .. ": " .. tostring(step.error or "department failed"),
      2
    )
  end
  return trace, step
end


local function mock_consensus_approval()
  t.mock_command(devloop_base.read_runtime_root_cmd(), {
    stdout = "/tmp/fkst-packages-test/github-devloop-integration/cross-repo-runtime",
    stderr = "",
    exit_code = 0,
  })
  for _, angle in ipairs({
    "teleology",
    "parsimony",
    "fidelity",
    "natural-ownership",
    "proportional-containment",
  }) do
    t.mock_command("test -d '" .. implementation_root .. "' && test -e '" .. implementation_root .. "'/.git", {
      stdout = "",
      stderr = "",
      exit_code = 0,
    })
    t.mock_command("codex exec", {
      stdout = verdict_label .. " approve\n" .. reply_label .. " " .. angle .. " approves.\n",
      stderr = "",
      exit_code = 0,
    })
  end
end

local function find_raise(step, queue, predicate)
  for _, raised in ipairs(step.raises or {}) do
    if raised.queue == queue and (predicate == nil or predicate(raised.payload, raised)) then
      return raised
    end
  end
  return nil
end

local function require_raise(step, queue, predicate)
  local raised = find_raise(step, queue, predicate)
  if raised == nil then
    local actual = {}
    for _, item in ipairs(step.raises or {}) do
      table.insert(actual, tostring(item.queue)
        .. ":handoff=" .. tostring(item.payload and item.payload.handoff and item.payload.handoff.kind))
    end
    error(
      "missing raised queue=" .. tostring(queue) .. " actual=" .. table.concat(actual, ","),
      2
    )
  end
  return raised
end

local function seed_review_context(review_proposal_id, review_dedup_key)
  local dir = os.tmpname() .. "-cross-repo-review-context"
  local mkdir_ok = os.execute("mkdir -p " .. devloop_base._shell_single_quote(dir))
  if not (mkdir_ok == true or mkdir_ok == 0) then
    error("failed to create review context fixture")
  end

  local bundle = { dir = dir }
  local files = {
    notice = { "notice_path", "UNTRUSTED-NOTICE.txt", "BEGIN UNTRUSTED BUNDLE DATA\nEND UNTRUSTED BUNDLE DATA\n" },
    issue = { "issue_path", "issue.json", "{}\n" },
    board = { "board_path", "board.txt", "No related work.\n" },
    pr = { "pr_path", "pr.json", "{}\n" },
    diff = { "diff_path", "diff.patch", "+return true\n" },
    risk = { "risk_path", "risk.txt", "PR risk tier: normal\n" },
  }
  for name, spec in pairs(files) do
    local path = dir .. "/" .. spec[2]
    local handle = assert(io.open(path, "w"))
    handle:write(spec[3])
    handle:close()
    bundle[spec[1]] = path
    bundle[name .. "_bytes"] = #spec[3]
    t.mock_command("wc -c < " .. devloop_base._shell_single_quote(path), {
      stdout = tostring(#spec[3]) .. "\n",
      stderr = "",
      exit_code = 0,
    })
  end

  local readable = {}
  for _, name in ipairs({ "notice", "issue", "board", "pr", "diff", "risk" }) do
    table.insert(readable, "test -r " .. devloop_base._shell_single_quote(bundle[name .. "_path"]))
  end
  t.mock_command(table.concat(readable, " && "), {
    stdout = "",
    stderr = "",
    exit_code = 0,
  })

  cache_set(context_bundle.context_bundle_key(review_proposal_id, review_dedup_key), dir)
  cache_set(
    context_bundle.context_bundle_manifest_key(review_proposal_id, review_dedup_key),
    context_bundle.context_bundle_manifest(bundle)
  )
  return function()
    os.execute("rm -rf " .. devloop_base._shell_single_quote(dir))
  end
end

local function review_result_payload(review_version, head_sha, decision, review_id)
  local selected_review_id = review_id
    or devloop_base.pr_review_proposal_id(implementation_repo, pr_number, review_version, head_sha)
  return {
    schema = "consensus.consensus_reached.v1",
    proposal_id = selected_review_id,
    decision = decision,
    body = decision == "approve"
      and "Review consensus approves the implementation diff."
      or "Review consensus rejects the implementation diff.",
    blocking_gap = decision == "reject" and "missing regression guard required by the issue" or nil,
    angle_results = {
      { angle = "minimal", verdict = decision },
      { angle = "structural", verdict = decision },
      { angle = "delete", verdict = decision },
    },
    dedup_key = "consensus:" .. selected_review_id .. "/review",
    source_ref = source_ref,
  }
end

local function run_cross_repo_lifecycle()
  mock_cross_repo_environment()

  local initial_pr_comments = {
    trusted(origin_marker(), "2026-07-27T01:04:00Z"),
    trusted(core.state_marker(proposal_id, "reviewing", root_version), "2026-07-27T01:04:01Z"),
  }
  mock_pr_selector(initial_pr_comments, old_head, entity_mocks.pr_origin_selector)
  local reject_write_reads = mock_write_modes({ "", "", "", "", "", "", "" })
  local _, reject_step = run_to_consumer(
    "consensus.consensus_reached",
    review_result_payload(root_version, old_head, "reject"),
    "github-devloop-pr.review_result",
    8
  )
  reject_write_reads()
  local reject_comment = require_raise(reject_step, "github-proxy.github_pr_comment_request")
  local reject_label = require_raise(reject_step, "github-proxy.github_issue_label_request")
  t.eq(reject_comment.payload.repo, implementation_repo)
  t.eq(tostring(reject_comment.payload.pr_number), tostring(pr_number))
  t.eq(reject_comment.payload.handoff.source_ref.ref, implementation_repo .. "#pr/" .. tostring(pr_number))
  t.eq(reject_label.payload.repo, lifecycle_repo)
  t.eq(tostring(reject_label.payload.issue_number), tostring(issue_number))
  t.eq(reject_label.payload.add_labels[1], "fkst-dev:fixing")

  local fixing_comments = {
    trusted(origin_marker(), "2026-07-27T01:04:00Z"),
    trusted(reject_comment.payload.body, "2026-07-27T01:05:00Z"),
  }
  entity_mocks.mock_pr_read_forms(t, {
    repo = implementation_repo,
    number = pr_number,
    head = branch,
    head_sha = old_head,
    base_branch = implementation_branch,
    base_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    head_repo = implementation_repo,
    state = "OPEN",
    comments = fixing_comments,
    labels = {},
    times = 1,
  })
  local replay_write_reads = mock_write_modes({ "", "", "", "", "", "", "" })
  local _, replay_step = run_to_consumer(
    "github-devloop-pr.devloop_observe_pr",
    {
      schema = "github-proxy.v1",
      type = "pr",
      repo = implementation_repo,
      number = pr_number,
      state = "open",
      updated_at = "2026-07-27T01:05:01Z",
      dedup_key = implementation_repo .. "#pr#7@2026-07-27T01:05:01Z",
      source_ref = source_ref,
    },
    "github-devloop-pr.observe_pr",
    8
  )
  replay_write_reads()
  local replay_comment = require_raise(replay_step, "github-proxy.github_pr_comment_request", function(payload)
    return payload.handoff ~= nil and payload.handoff.kind == "github-devloop.fixing"
  end)
  t.eq(replay_comment.payload.repo, implementation_repo)
  t.eq(replay_comment.payload.handoff.source_ref.ref, implementation_repo .. "#pr/" .. tostring(pr_number))

  local durable_fixing_comments = {
    trusted(origin_marker(), "2026-07-27T01:04:00Z"),
    trusted(reject_comment.payload.body, "2026-07-27T01:05:00Z"),
    trusted(replay_comment.payload.body, "2026-07-27T01:05:02Z"),
  }
  mock_pr_selector(durable_fixing_comments, new_head, entity_mocks.pr_fix_selector)
  mock_command_repeated(
    "git -C '" .. implementation_root .. "' rev-parse --verify refs/heads/" .. branch,
    { stdout = new_head .. "\n" },
    2
  )
  local fixing_comment_id = "IC_cross_repo_fixing_1"
  t.mock_command(
    "gh api --method GET 'repos/" .. implementation_repo .. "/issues/comments/" .. fixing_comment_id .. "'",
    {
      stdout = '{"body":"' .. h.json_string(replay_comment.payload.body)
        .. '","user":{"login":"' .. core._test_bot_login .. '"}}\n',
      stderr = "",
      exit_code = 0,
    }
  )
  local codex_calls_before_fix = h.count_calls("codex exec")
  local fixing_write_reads = mock_write_modes({ "", "", "", "", "", "", "", "", "" })
  local handoff_trace, handoff_step = run_to_consumer(
    "github-proxy.github_comment_written",
    {
      schema = "github-proxy.comment-written.v1",
      repo = implementation_repo,
      target = "pr",
      pr_number = pr_number,
      comment_id = fixing_comment_id,
      request_dedup_key = replay_comment.payload.dedup_key,
      dedup_key = replay_comment.payload.dedup_key .. "/written/" .. fixing_comment_id,
      source_ref = source_ref,
      handoff = replay_comment.payload.handoff,
    },
    "github-devloop-pr.comment_handoff",
    8
  )
  fixing_write_reads()
  local replayed_fix = require_raise(handoff_step, "github-devloop-pr.devloop_fixing").payload
  t.eq(replayed_fix.proposal_id, proposal_id)
  t.eq(replayed_fix.source_ref.ref, implementation_repo .. "#pr/" .. tostring(pr_number))
  t.eq(replayed_fix.reviewed_head_sha, old_head)

  local fix_step = graph.require_delivery(handoff_trace, {
    queue = "github-devloop-pr.devloop_fixing",
    consumer = "github-devloop-pr.fix",
  })
  if fix_step.status ~= "accepted" or fix_step.exit_code ~= 0 then
    error("cross-repo fix delivery failed: " .. tostring(fix_step.error or "department failed"), 2)
  end
  local fixed_comment = require_raise(fix_step, "github-proxy.github_pr_comment_request")
  local fixed_label = require_raise(fix_step, "github-proxy.github_issue_label_request")
  t.eq(fixed_comment.payload.repo, implementation_repo)
  t.eq(tostring(fixed_comment.payload.pr_number), tostring(pr_number))
  t.eq(fixed_comment.payload.handoff.kind, "github-devloop.reviewing")
  t.eq(fixed_comment.payload.handoff.source_ref.ref, implementation_repo .. "#pr/" .. tostring(pr_number))
  t.eq(fixed_label.payload.repo, lifecycle_repo)
  t.eq(tostring(fixed_label.payload.issue_number), tostring(issue_number))
  t.eq(h.count_calls("codex exec"), codex_calls_before_fix)
  t.eq(h.count_calls("git push origin"), 0)

  local reviewing_version = fixed_comment.payload.handoff.version
  local review_id = devloop_base.pr_review_proposal_id(
    implementation_repo,
    pr_number,
    reviewing_version,
    new_head
  )
  local review_dedup_key = devloop_base.pr_review_proposal_dedup_key(review_id)
  local cleanup_context = seed_review_context(review_id, review_dedup_key)
  local reviewing_comments = {
    trusted(origin_marker(), "2026-07-27T01:04:00Z"),
    trusted(reject_comment.payload.body, "2026-07-27T01:05:00Z"),
    trusted(replay_comment.payload.body, "2026-07-27T01:05:02Z"),
    trusted(fixed_comment.payload.body, "2026-07-27T01:06:00Z"),
  }
  mock_pr_selector(reviewing_comments, new_head, entity_mocks.pr_origin_selector)
  mock_pr_selector(reviewing_comments, new_head, entity_mocks.pr_origin_selector)
  entity_mocks.mock_issue_view_selector(t, {
    repo = lifecycle_repo,
    number = issue_number,
    title = "Deliver cron support through the implementation repository",
    labels = { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
    comments = lifecycle_comments(),
    assignees = { core._test_bot_login },
    author_login = core._test_bot_login,
  }, "title,labels,comments,assignees,author")
  mock_command_repeated(
    "gh pr diff '" .. tostring(pr_number) .. "' --repo '" .. implementation_repo .. "' --name-only",
    { stdout = "file.lua\n" },
    4
  )
  local reviewing_comment_id = "IC_cross_repo_reviewing_1"
  mock_command_repeated(
    "gh api --method GET 'repos/" .. implementation_repo .. "/issues/comments/" .. reviewing_comment_id .. "'",
    {
      stdout = '{"body":"' .. h.json_string(fixed_comment.payload.body)
        .. '","user":{"login":"' .. core._test_bot_login .. '"}}\n',
    },
    2
  )
  mock_consensus_approval()
  local reviewing_write_reads = mock_write_modes({
    "", "", "", "", "", "", "", "", "", "",
    "", "", "",
  })
  local review_ok, review_trace, review_step = pcall(function()
    local trace, handoff_step = run_to_consumer(
      "github-proxy.github_comment_written",
      {
        schema = "github-proxy.comment-written.v1",
        repo = implementation_repo,
        target = "pr",
        pr_number = pr_number,
        comment_id = reviewing_comment_id,
        request_dedup_key = fixed_comment.payload.dedup_key,
        dedup_key = fixed_comment.payload.dedup_key .. "/written/" .. reviewing_comment_id,
        source_ref = source_ref,
        handoff = fixed_comment.payload.handoff,
      },
      "github-devloop-pr.comment_handoff",
      12
    )
    local reviewing_raise = require_raise(handoff_step, "github-devloop-pr.devloop_reviewing")
    t.eq(reviewing_raise.payload.proposal_id, proposal_id)
    t.eq(reviewing_raise.payload.source_ref.ref, implementation_repo .. "#pr/" .. tostring(pr_number))
    local step = graph.require_delivery(trace, {
      queue = "github-devloop-pr.devloop_reviewing",
      consumer = "github-devloop-pr.review_pr",
    })
    if step.status ~= "accepted" or step.exit_code ~= 0 then
      error("cross-repo review delivery failed: " .. tostring(step.error or "department failed"), 2)
    end
    return trace, step
  end)
  cleanup_context()
  if not review_ok then
    error(review_trace, 0)
  end
  reviewing_write_reads()
  local review_proposal = require_raise(review_step, "consensus.proposal").payload
  t.eq(review_proposal.proposal_id, review_id)
  t.eq(review_proposal.source_ref.ref, implementation_repo .. "#pr/" .. tostring(pr_number))
  t.eq(review_proposal.worktree, implementation_root)
  t.is_true(review_proposal.body:find("Entity proposal: " .. proposal_id, 1, true) ~= nil)
  t.is_true(review_proposal.title:find("Deliver cron support through the implementation repository", 1, true) ~= nil)
  t.is_true(h.count_calls("gh issue view '42' --repo '" .. lifecycle_repo .. "'") > 0)
  t.is_true(h.count_calls("gh pr view '7' --repo '" .. implementation_repo .. "'") > 0)

  local approve_step = graph.require_delivery(review_trace, {
    queue = "consensus.consensus_reached",
    consumer = "github-devloop-pr.review_result",
  })
  local approve_comment = require_raise(approve_step, "github-proxy.github_pr_comment_request", function(payload)
    return payload.handoff ~= nil and payload.handoff.kind == "github-devloop.merge_ready"
  end)
  local approve_label = require_raise(approve_step, "github-proxy.github_issue_label_request")
  t.eq(approve_comment.payload.repo, implementation_repo)
  t.eq(approve_label.payload.repo, lifecycle_repo)
  t.eq(approve_label.payload.add_labels[1], "fkst-dev:merge-ready")

  local merge_ready = payloads_builders.build_devloop_merge_ready_payload(
    proposal_id,
    pr_number,
    approve_comment.payload.handoff.version,
    {
      review_proposal_id = approve_comment.payload.handoff.review_proposal_id,
      review_dedup_key = approve_comment.payload.handoff.review_dedup_key,
      reviewed_head_sha = approve_comment.payload.handoff.reviewed_head_sha,
      current_head_sha = approve_comment.payload.handoff.current_head_sha,
    },
    source_ref
  )
  local merge_comments = {
    trusted(origin_marker(), "2026-07-27T01:04:00Z"),
    trusted(reject_comment.payload.body, "2026-07-27T01:05:00Z"),
    trusted(fixed_comment.payload.body, "2026-07-27T01:06:00Z"),
    trusted(approve_comment.payload.body, "2026-07-27T01:07:00Z"),
  }
  local merge_fields = {
    repo = implementation_repo,
    number = pr_number,
    head = branch,
    head_sha = new_head,
    base_branch = implementation_branch,
    base_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    head_repo = implementation_repo,
    state = "OPEN",
    comments = merge_comments,
    labels = {},
    mergeable = "MERGEABLE",
    merge_state = "CLEAN",
    status_check_rollup_json = '[{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS","startedAt":"2026-07-27T01:07:01Z","completedAt":"2026-07-27T01:08:01Z"}]',
  }
  entity_mocks.mock_pr_view_selector(t, merge_fields, entity_mocks.pr_merge_selector, 3)
  local merged_comments = {
    trusted(origin_marker(), "2026-07-27T01:04:00Z"),
    trusted(reject_comment.payload.body, "2026-07-27T01:05:00Z"),
    trusted(fixed_comment.payload.body, "2026-07-27T01:06:00Z"),
    trusted(approve_comment.payload.body, "2026-07-27T01:07:00Z"),
    trusted(core.state_marker(proposal_id, "merging", merge_ready.version), "2026-07-27T01:08:02Z"),
    trusted(m_builders.merging_marker(
      proposal_id,
      pr_number,
      merge_ready.version,
      new_head
    ), "2026-07-27T01:08:02Z"),
  }
  local merged_fields = {}
  for key, value in pairs(merge_fields) do
    merged_fields[key] = value
  end
  merged_fields.state = "MERGED"
  merged_fields.comments = merged_comments
  merged_fields.merge_commit_sha = merge_commit
  merged_fields.merged_at = "2026-07-27T02:03:04Z"
  entity_mocks.mock_pr_view_selector(t, merged_fields, entity_mocks.pr_merge_selector, 1)
  local merge_write_reads = mock_write_modes({ "", "", "1", "", "" })
  t.mock_command(
    "gh api --paginate --slurp 'repos/" .. implementation_repo
      .. "/pulls?state=open&base=" .. implementation_branch .. "&per_page=100'",
    { stdout = "[[]]\n", stderr = "", exit_code = 0 }
  )
  t.mock_command("gh pr comment '7' --repo '" .. implementation_repo .. "' --body-file", {
    stdout = "commented\n",
    stderr = "",
    exit_code = 0,
  })
  t.mock_command(
    "gh pr merge '7' --repo '" .. implementation_repo .. "' --merge --match-head-commit '" .. new_head .. "'",
    { stdout = "merged\n", stderr = "", exit_code = 0 }
  )
  local _, merge_step = run_to_consumer(
    "github-devloop-pr.devloop_merge_ready",
    merge_ready,
    "github-devloop-pr.merge"
  )
  merge_write_reads()
  local merged_comment = require_raise(merge_step, "github-proxy.github_pr_comment_request", function(payload)
    return tostring(payload.body or ""):find("fkst:github-devloop:merged:v1", 1, true) ~= nil
  end)
  t.eq(merged_comment.payload.repo, implementation_repo)
  t.eq(tostring(merged_comment.payload.pr_number), tostring(pr_number))
  t.eq(h.count_calls("gh pr merge '7' --repo '" .. implementation_repo .. "'"), 1)
  t.eq(h.count_calls("gh pr merge '7' --repo '" .. lifecycle_repo .. "'"), 0)

  local parent_comments = lifecycle_comments()
  entity_mocks.mock_issue_read_forms(t, {
    repo = lifecycle_repo,
    number = issue_number,
    title = "Deliver cron support through the implementation repository",
    body = "Lifecycle issue body",
    state = "OPEN",
    updated_at = "2026-07-27T02:04:00Z",
    labels = { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
    comments = parent_comments,
    assignees = { core._test_bot_login },
    author_login = core._test_bot_login,
    times = 4,
  })
  entity_mocks.mock_issue_view_selector(t, {
    repo = lifecycle_repo,
    number = issue_number,
    title = "Deliver cron support through the implementation repository",
    state = "OPEN",
    updated_at = "2026-07-27T02:04:00Z",
    labels = { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
    comments = parent_comments,
    assignees = { core._test_bot_login },
    author_login = core._test_bot_login,
  }, "title,createdAt,updatedAt,labels,state,comments,assignees,author")
  local terminal_pr_comments = {
    trusted(origin_marker(), "2026-07-27T01:04:00Z"),
    trusted(merged_comment.payload.body, "2026-07-27T02:03:05Z"),
  }
  mock_pr_origin(terminal_pr_comments, new_head, "MERGED", 4)
  t.mock_command("gh issue close '42' --repo '" .. lifecycle_repo .. "'", {
    stdout = "closed\n",
    stderr = "",
    exit_code = 0,
  })
  local terminal_write_reads = mock_write_modes({
    "", "", "", "", "1", "", "", "", "",
  })
  local _, terminal_step = run_to_consumer(
    "github-devloop.devloop_observe_issue",
    {
      schema = "github-proxy.v1",
      type = "issue",
      repo = lifecycle_repo,
      number = issue_number,
      title = "Deliver cron support through the implementation repository",
      state = "OPEN",
      updated_at = "2026-07-27T02:04:00Z",
      labels = { "fkst-dev:enabled", "fkst-dev:awaiting-pr" },
      dedup_key = lifecycle_repo .. "#issue#42@2026-07-27T02:04:00Z",
      source_ref = entity_lib.issue_source_ref(lifecycle_repo, issue_number),
    },
    "github-devloop.observe_issue"
  )
  terminal_write_reads()
  t.eq(h.count_calls("gh issue close '42' --repo '" .. lifecycle_repo .. "'"), 1)
  t.eq(h.count_calls("gh issue close '42' --repo '" .. implementation_repo .. "'"), 0)
  local terminal_comment = require_raise(terminal_step, "github-proxy.github_issue_comment_request")
  local terminal_label = require_raise(terminal_step, "github-proxy.github_issue_label_request")
  t.eq(terminal_comment.payload.repo, lifecycle_repo)
  t.eq(tostring(terminal_comment.payload.issue_number), tostring(issue_number))
  t.eq(terminal_label.payload.repo, lifecycle_repo)
  t.eq(tostring(terminal_label.payload.issue_number), tostring(issue_number))
  t.is_true(terminal_comment.payload.body:find('state="merged"', 1, true) ~= nil)
end

return {
  test_cross_repo_delivery_recovers_rejection_then_reviews_merges_and_closes_only_lifecycle_issue = function()
    run_cross_repo_lifecycle()
  end,
}
