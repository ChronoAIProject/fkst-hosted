local h = require("tests.devloop_helpers")
local t = h.t
local core = h.core
local issue = h.issue
local reached = h.reached
local decompose_event = h.decompose_event
local opts = h.opts
local source_ref = h.source_ref
local run_observe = h.run_observe
local run_implement = h.run_implement
local run_review_pr = h.run_review_pr
local mock_issue_state = h.mock_issue_state
local mock_issue_implement_raw = h.mock_issue_implement_raw
local mock_issue_review = h.mock_issue_review
local count_calls = h.count_calls
local find_raise = h.find_raise
local render_comment = h.render_comment
local json_string = h.json_string

local function has_value(values, expected)
  for _, value in ipairs(values or {}) do
    if value == expected then
      return true
    end
  end
  return false
end

local function mock_linked_pr_state(comments, state, exit_code, times)
  local rendered_comments = {}
  for _, comment in ipairs(comments or {}) do
    table.insert(rendered_comments, render_comment(comment))
  end
  local stderr = ""
  if exit_code ~= nil and exit_code ~= 0 then
    stderr = "pr view failed"
  end
  for _ = 1, times or 1 do
    t.mock_command("--json headRefName,headRefOid,baseRefName,state,updatedAt,comments", {
      stdout = string.format(
        '{"headRefName":"devloop-owner-repo-42-01HY","headRefOid":"def456","baseRefName":"dev","state":"%s","updatedAt":"2026-06-03T02:03:04Z","comments":[%s]}\n',
        json_string(state or "OPEN"),
        table.concat(rendered_comments, ",")
      ),
      stderr = stderr,
      exit_code = exit_code or 0,
    })
  end
end

local function mock_decompose_child_issue_list(event, indexes)
  local rendered = {}
  for _, index in ipairs(indexes or {}) do
    table.insert(rendered, string.format(
      '{"number":%d,"title":"Child %d","state":"OPEN","author":{"login":"fkst-test-bot"},"body":"%s","url":"https://github.example/owner/repo/issues/%d"}',
      100 + index,
      index,
      json_string(core.decompose_child_marker(event.proposal_id, event.version, event.pr_number, index)),
      100 + index
    ))
  end
  t.mock_command(core.gh_issue_list_decompose_children_cmd("owner/repo", event.proposal_id), {
    stdout = "[" .. table.concat(rendered, ",") .. "]\n",
    stderr = "",
    exit_code = 0,
  })
end

local function merge_gate_fix_marker(event)
  return core.merge_gate_marker(
    event.proposal_id,
    event.pr_number,
    event.version,
    event.review_proposal_id,
    event.review_dedup_key,
    event.head_sha,
    nil,
    "rollup-red"
  )
end

local function fresh_thinking_marker(proposal_id, version)
  return {
    body = core.state_marker(proposal_id, "thinking", version),
    created_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now()),
  }
end

return {
  test_observe_issue_reraises_thinking_proposal_for_poll_self_heal = function()
    local event = issue()
    local original = core.build_proposal(event)
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", {
      fresh_thinking_marker(original.proposal_id, original.dedup_key),
    })

    local first = run_observe(event, opts("observe-issue-thinking-self-heal-1"))
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 1)
    local first_proposal = find_raise(first.raises, "consensus.proposal").payload
    t.eq(first_proposal.schema, "consensus.proposal.v1")
    t.eq(first_proposal.proposal_id, original.proposal_id)
    t.eq(first_proposal.dedup_key, original.dedup_key .. "/replay")
    t.eq(first_proposal.source_ref.ref, "owner/repo#issue/42")

    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", {
      fresh_thinking_marker(original.proposal_id, original.dedup_key),
    })
    local same = run_observe(event, opts("observe-issue-thinking-self-heal-same-fact"))
    t.eq(same.exit_code, 0)
    t.eq(#same.raises, 1)
    local same_proposal = find_raise(same.raises, "consensus.proposal").payload
    t.eq(same_proposal.dedup_key, first_proposal.dedup_key)
    t.eq(same_proposal.content_fetch, first_proposal.content_fetch)

    local updated_event = issue({ updated_at = "2026-06-03T01:02:04Z" })
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", {
      fresh_thinking_marker(original.proposal_id, original.dedup_key),
    })
    local second = run_observe(updated_event, opts("observe-issue-thinking-self-heal-2"))
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 1)
    local second_proposal = find_raise(second.raises, "consensus.proposal").payload
    t.eq(second_proposal.dedup_key, core.build_proposal(updated_event).dedup_key .. "/replay")
    t.is_true(second_proposal.dedup_key ~= first_proposal.dedup_key)
    t.is_true(second_proposal.content_fetch ~= first_proposal.content_fetch)
    t.eq(count_calls("--json title,body,comments,labels,state,updatedAt,assignees"), 3)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_replays_mid_loop_thinking_proposal_from_converge_marker = function()
    local event = issue()
    local original = core.build_proposal(event)
    local base_version = original.dedup_key
    local sr_digest = core.source_ref_digest(event.source_ref)
    local angle_digests = {
      { angle = "minimal", verdict = "abstain", digest = "needs-narrower-scope" },
    }
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", {
      fresh_thinking_marker(original.proposal_id, base_version),
      core.converge_round_marker(original.proposal_id, base_version, sr_digest, 0, base_version, "Narrow the question", angle_digests),
    })

    local result = run_observe(event, opts("observe-issue-thinking-mid-loop-self-heal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local proposal = find_raise(result.raises, "consensus.proposal").payload
    t.eq(proposal.dedup_key, core.build_proposal(event).dedup_key .. "/loop/1")
    t.eq(proposal.round, 1)
    t.eq(proposal.convergence_question, "Narrow the question")
    t.eq(proposal.prior_round_digests[1].digest, "needs-narrower-scope")
    t.eq(count_calls("--json title,body,comments,labels,state,updatedAt,assignees"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_replays_thinking_base_proposal_when_converge_marker_is_missing = function()
    local event = issue()
    local original = core.build_proposal(event)
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", {
      {
        body = core.state_marker(original.proposal_id, "thinking", original.dedup_key .. "/loop/1"),
        created_at = os.date("!%Y-%m-%dT%H:%M:%SZ", now()),
      },
    })

    local result = run_observe(event, opts("observe-issue-thinking-missing-converge", {
      now = "2026-06-03T02:00:00Z",
    }))
    t.eq(result.exit_code, 0)
    local proposal = find_raise(result.raises, "consensus.proposal").payload
    t.eq(proposal.proposal_id, original.proposal_id)
    t.eq(proposal.dedup_key, original.dedup_key .. "/replay/loop/1")
    t.eq(proposal.source_ref.ref, "owner/repo#issue/42")
  end,

  test_observe_issue_timeout_redrives_plain_thinking_before_replay = function()
    local event = issue()
    local original = core.build_proposal(event)
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:thinking" }, "OPEN", {
      {
        body = core.state_marker(original.proposal_id, "thinking", original.dedup_key),
        created_at = "2026-06-03T00:00:00Z",
      },
    })

    local result = run_observe(event, opts("observe-issue-thinking-timeout-redrive", {
      now = "2026-06-03T02:00:00Z",
    }))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local proposal = find_raise(result.raises, "consensus.proposal").payload
    t.eq(proposal.proposal_id, original.proposal_id)
    t.eq(proposal.dedup_key, original.dedup_key .. "/replay/timeout/thinking/1")
    t.eq(proposal.source_ref.ref, "owner/repo#issue/42")
  end,

  test_observe_issue_reraises_ready_for_poll_self_heal = function()
    local event = reached()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:ready" }, "OPEN", {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:ready" } }), opts("observe-issue-ready-self-heal"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 1)
    local ready_raise = find_raise(result.raises, "devloop_ready")
    t.eq(ready_raise.payload.schema, "github-devloop.ready.v1")
    t.eq(ready_raise.payload.proposal_id, event.proposal_id)
    t.eq(ready_raise.payload.source_ref.ref, "owner/repo#issue/42")
    t.eq(ready_raise.payload.dedup_key, core.build_devloop_ready_payload({
      proposal_id = event.proposal_id,
      dedup_key = event.dedup_key,
      source_ref = event.source_ref,
    }).dedup_key)
    t.is_nil(ready_raise.payload.ready_hand_off)
    t.eq(count_calls("--json title,body,comments,labels,state,updatedAt,assignees"), 1)
    t.eq(count_calls("--json body"), 0)
  end,

  test_observe_issue_ready_self_heal_does_not_duplicate_after_implementing = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    local branch = core.implement_branch("owner/repo", 42, ready_payload.dedup_key)
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:implementing" }, "OPEN", {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
      core.state_marker(event.proposal_id, "implementing", ready_payload.dedup_key),
      core.implement_attempt_marker(event.proposal_id, ready_payload.dedup_key, 1, tostring(now())),
    })

    local observed = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:implementing" } }), opts("observe-issue-ready-self-heal-advanced"))
    t.eq(observed.exit_code, 0)
    t.eq(find_raise(observed.raises, "devloop_ready"), nil)
    t.eq(count_calls("--json title,body,comments,labels,state,updatedAt,assignees"), 1)
    t.eq(count_calls("--json body"), 0)

    mock_issue_implement_raw({ "fkst-dev:implementing" }, {
      core.state_marker(event.proposal_id, "ready", event.dedup_key),
      core.state_marker(event.proposal_id, "implementing", ready_payload.dedup_key),
      core.implement_attempt_marker(event.proposal_id, ready_payload.dedup_key, 1, tostring(now())),
      core.implementing_marker(event.proposal_id, ready_payload.dedup_key, branch, "abc123", "dev", "def456"),
      core.pr_link_marker(event.proposal_id, 7, branch, ready_payload.dedup_key, "dev"),
    })
    local implemented = run_implement(ready_payload, opts("implement-ready-self-heal-advanced"))
    t.eq(implemented.exit_code, 0)
    t.eq(#implemented.raises, 0)
  end,

  test_observe_issue_uses_pr_local_current_state_over_issue_pr_open = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    local issue_comments = {
      core.state_marker(event.proposal_id, "pr-open", ready_payload.dedup_key),
      core.pr_link_marker(event.proposal_id, 7, "devloop-owner-repo-42-01HY", ready_payload.dedup_key, "dev"),
    }
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", issue_comments)
    mock_linked_pr_state({
      core.state_marker(event.proposal_id, "reviewing", ready_payload.dedup_key),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" } }), opts("observe-issue-pr-local-reviewing"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:reviewing")
    t.is_true(has_value(label_raise.payload.remove_labels, "fkst-dev:pr-open"))
    t.eq(count_calls("--json title,body,comments,labels,state,updatedAt,assignees"), 1)
    t.eq(count_calls("--json headRefName,headRefOid,baseRefName,state,updatedAt,comments"), 1)
  end,

  test_observe_issue_pr_open_reraises_reviewing_for_poll_self_heal = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    local comments = {
      core.state_marker(event.proposal_id, "pr-open", ready_payload.dedup_key),
      core.pr_link_marker(event.proposal_id, 7, "devloop-owner-repo-42-01HY", ready_payload.dedup_key, "dev"),
    }
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", comments)
    mock_linked_pr_state({}, nil, nil, 2)

    local first = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" } }), opts("observe-issue-pr-open-review-kickoff-1"))
    t.eq(first.exit_code, 0)
    t.eq(#first.raises, 2)
    local first_comment = find_raise(first.raises, "github-proxy.github_pr_comment_request")
    t.is_true(first_comment.payload.body:find(core.state_marker(event.proposal_id, "reviewing", ready_payload.dedup_key), 1, true) ~= nil)
    local first_reviewing = find_raise(first.raises, "devloop_reviewing")
    t.eq(first_reviewing.payload.schema, "github-devloop.reviewing.v1")
    t.eq(first_reviewing.payload.proposal_id, event.proposal_id)
    t.eq(first_reviewing.payload.pr_number, 7)
    t.eq(first_reviewing.payload.version, ready_payload.dedup_key)
    t.eq(first_reviewing.payload.source_ref.ref, "owner/repo#pr/7")

    mock_issue_review({ "fkst-dev:reviewing" }, {
      first_comment.payload.body,
    })
    local review = run_review_pr(first_reviewing.payload, opts("observe-issue-pr-open-review-kickoff-review-pr"))
    t.eq(review.exit_code, 0)
    t.eq(#review.raises, 1)
    local proposal = find_raise(review.raises, "consensus.proposal").payload
    t.eq(proposal.proposal_id, core.pr_review_proposal_id("owner/repo", 7, ready_payload.dedup_key, "def456"))

    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", comments)
    mock_linked_pr_state({}, nil, nil, 2)
    local second = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" } }), opts("observe-issue-pr-open-review-kickoff-2"))
    t.eq(second.exit_code, 0)
    t.eq(#second.raises, 2)
    local second_reviewing = find_raise(second.raises, "devloop_reviewing")
    t.eq(second_reviewing.payload.dedup_key, first_reviewing.payload.dedup_key)
  end,

  test_observe_issue_pr_open_timeout_redrive_reaches_reviewing = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    local comments = {
      {
        body = core.state_marker(event.proposal_id, "pr-open", ready_payload.dedup_key),
        created_at = "2026-06-03T01:00:00Z",
      },
      core.pr_link_marker(event.proposal_id, 7, "devloop-owner-repo-42-01HY", ready_payload.dedup_key, "dev"),
    }
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", comments)
    mock_linked_pr_state({}, nil, nil, 2)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" }, source = "liveness-scan" }), opts("observe-issue-pr-open-timeout-redrive", {
      now = "2026-06-03T03:00:00Z",
    }))
    t.eq(result.exit_code, 0)
    local reviewing = find_raise(result.raises, "devloop_reviewing")
    t.is_true(reviewing ~= nil)
    t.eq(reviewing.payload.version, ready_payload.dedup_key .. "/review-loop/1")
    t.eq(reviewing.payload.source_ref.ref, "owner/repo#pr/7")

    mock_issue_review({ "fkst-dev:reviewing" }, {
      core.state_marker(event.proposal_id, "reviewing", reviewing.payload.version),
    })
    local review = run_review_pr(reviewing.payload, opts("observe-issue-pr-open-timeout-review-pr"))
    t.eq(review.exit_code, 0)
    local proposal = find_raise(review.raises, "consensus.proposal")
    t.is_true(proposal ~= nil)
    t.eq(proposal.payload.proposal_id, core.pr_review_proposal_id("owner/repo", 7, reviewing.payload.version, "def456"))
  end,

  test_observe_issue_pr_open_closed_link_redrives_ready_for_replacement_pr = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    local comments = {
      {
        body = core.state_marker(event.proposal_id, "pr-open", ready_payload.dedup_key),
        created_at = "2026-06-03T01:00:00Z",
      },
      core.pr_link_marker(event.proposal_id, 7, "devloop-owner-repo-42-01HY", ready_payload.dedup_key, "dev"),
    }
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", comments)
    mock_linked_pr_state({}, "CLOSED", nil, 2)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" }, source = "liveness-scan" }), opts("observe-issue-pr-open-closed-link-redrive", {
      now = "2026-06-03T03:00:00Z",
    }))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    local ready = find_raise(result.raises, "devloop_ready")
    t.is_true(ready ~= nil)
    t.eq(ready.payload.dedup_key, "ready/" .. ready_payload.dedup_key .. "/reimplement/1")
    t.eq(ready.payload.source_ref.ref, "owner/repo#issue/42")
  end,

  test_observe_issue_pr_open_absent_link_redrives_ready_for_replacement_pr = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    local comments = {
      {
        body = core.state_marker(event.proposal_id, "pr-open", ready_payload.dedup_key),
        created_at = "2026-06-03T01:00:00Z",
      },
      core.pr_link_marker(event.proposal_id, 7, "devloop-owner-repo-42-01HY", ready_payload.dedup_key, "dev"),
    }
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", comments)
    mock_linked_pr_state({}, "OPEN", 1)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" }, source = "liveness-scan" }), opts("observe-issue-pr-open-absent-link-redrive", {
      now = "2026-06-03T03:00:00Z",
    }))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)

    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", comments)
    for _ = 1, 2 do
      t.mock_command("--json headRefName,headRefOid,baseRefName,state,updatedAt,comments", {
        stdout = "",
        stderr = "HTTP 404: Not Found\n",
        exit_code = 1,
      })
    end

    local absent = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" }, source = "liveness-scan" }), opts("observe-issue-pr-open-absent-link-known-missing", {
      now = "2026-06-03T03:00:00Z",
    }))
    t.eq(absent.exit_code, 0)
    t.eq(find_raise(absent.raises, "devloop_reviewing"), nil)
    local ready = find_raise(absent.raises, "devloop_ready")
    t.is_true(ready ~= nil)
    t.eq(ready.payload.dedup_key, "ready/" .. ready_payload.dedup_key .. "/reimplement/1")
    t.eq(ready.payload.source_ref.ref, "owner/repo#issue/42")
  end,

  test_observe_issue_pr_open_reraise_requires_matching_link_version = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", {
      core.state_marker(event.proposal_id, "pr-open", ready_payload.dedup_key .. "/other"),
      core.pr_link_marker(event.proposal_id, 7, "devloop-owner-repo-42-01HY", ready_payload.dedup_key, "dev"),
    })
    mock_linked_pr_state({}, nil, nil, 2)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" } }), opts("observe-issue-pr-open-review-kickoff-version-mismatch"))
    t.eq(result.exit_code, 0)
    t.eq(#result.raises, 0)
  end,

  test_observe_issue_fixing_replay_accepts_premigration_pr_link_lineage = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local base = "ready/consensus-github-devloop/issue/owner/repo/42/185/2026-06-10T13-45-26Z"
    local issue_version = base .. "/fix/1/fix/2/fix/3/fix/4/fix/5"
    local link_version = base .. "/fix/1/review-loop/2/rereview/2/feedface"
    local review_proposal = core.pr_review_proposal_id("owner/repo", 7, core._strip_latest_fix_version_suffix(issue_version), "def456")
    local review_dedup = "consensus:" .. review_proposal .. "/review"
    local feedback = core.build_review_result_comment_request("owner/repo", 42, proposal_id, issue_version, {
      proposal_id = review_proposal,
      decision = "reject",
      body = "Review consensus rejects the diff.",
      blocking_gap = "missing regression guard",
      dedup_key = review_dedup,
      source_ref = core.pr_source_ref("owner/repo", 7),
    }, core.pr_source_ref("owner/repo", 7)).body
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:fixing" }, "OPEN", {
      core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", link_version, "dev"),
      core.state_marker(proposal_id, "fixing", issue_version),
      feedback,
    })
    mock_linked_pr_state({}, nil, nil, 2)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:fixing" } }), opts("observe-issue-fixing-premigration-link"))
    t.eq(result.exit_code, 0)
    local fixing_raise = find_raise(result.raises, "devloop_fixing")
    t.eq(fixing_raise.payload.version, issue_version)
    t.eq(fixing_raise.payload.review_proposal_id, review_proposal)
  end,

  test_observe_issue_fixing_replay_refuses_cross_proposal_pr_link = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local other_proposal_id = "github-devloop/issue/owner/repo/43"
    local base = "ready/consensus-github-devloop/issue/owner/repo/42/185/2026-06-10T13-45-26Z"
    local issue_version = base .. "/fix/1/fix/2"
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:fixing" }, "OPEN", {
      core.pr_link_marker(other_proposal_id, 7, "devloop-owner-repo-43-01HY", base, "dev"),
      core.state_marker(proposal_id, "fixing", issue_version),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:fixing" } }), opts("observe-issue-fixing-cross-proposal-link"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_fixing"), nil)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
  end,

  test_observe_issue_timeout_redrives_issue_local_non_replay_state = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:reviewing" }, "OPEN", {
      core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z", "dev"),
      {
        body = core.state_marker(proposal_id, "reviewing", version),
        created_at = "2026-06-03T01:00:00Z",
      },
    })
    mock_linked_pr_state({}, nil, nil, 2)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:reviewing" } }), opts("observe-issue-reviewing-timeout-redrive"))
    t.eq(result.exit_code, 0)
    local reviewing = find_raise(result.raises, "devloop_reviewing")
    t.is_true(reviewing ~= nil)
    t.eq(reviewing.payload.version, version .. "/review-loop/1")
    t.eq(reviewing.payload.source_ref.ref, "owner/repo#pr/7")
    local comment = find_raise(result.raises, "github-proxy.github_pr_comment_request")
    t.is_true(comment ~= nil)

    mock_issue_review({ "fkst-dev:reviewing" }, {
      comment.payload.body,
    })
    local review = run_review_pr(reviewing.payload, opts("observe-issue-reviewing-timeout-review-pr"))
    t.eq(review.exit_code, 0)
    t.is_true(find_raise(review.raises, "consensus.proposal") ~= nil)
  end,

  test_observe_issue_pr_open_does_not_reraise_after_pr_local_reviewing = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", {
      core.state_marker(event.proposal_id, "pr-open", ready_payload.dedup_key),
      core.pr_link_marker(event.proposal_id, 7, "devloop-owner-repo-42-01HY", ready_payload.dedup_key, "dev"),
    })
    mock_linked_pr_state({
      core.state_marker(event.proposal_id, "reviewing", ready_payload.dedup_key),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" } }), opts("observe-issue-pr-open-reviewing-no-reraise"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
  end,

  test_observe_issue_reconciles_blocked_terminal_label_from_marker = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/13"
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:reviewing" }, "OPEN", {
      core.state_marker(proposal_id, "blocked", version),
    })
    mock_decompose_child_issue_list({ proposal_id = proposal_id }, {})

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:reviewing" } }), opts("observe-issue-terminal-label-reconcile-blocked"))
    t.eq(result.exit_code, 0)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.is_true(label_raise ~= nil)
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:blocked")
    t.is_true(has_value(label_raise.payload.remove_labels, "fkst-dev:reviewing"))
    t.eq(label_raise.payload.source_ref.ref, "owner/repo#issue/42")
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    t.eq(count_calls("--json title,body,comments,labels,state,updatedAt,assignees"), 1)
    t.eq(count_calls("--json headRefName,headRefOid,baseRefName,state,updatedAt,comments"), 0)
  end,

  test_observe_issue_reconciles_issue_terminal_label_over_linked_pr_state = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z/fix/13"
    local link_version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:reviewing" }, "OPEN", {
      core.pr_link_marker(proposal_id, 7, "devloop-owner-repo-42-01HY", link_version, "dev"),
      core.state_marker(proposal_id, "blocked", version),
    })
    mock_linked_pr_state({
      core.state_marker(proposal_id, "reviewing", link_version),
    })
    mock_decompose_child_issue_list({ proposal_id = proposal_id }, {})

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:reviewing" } }), opts("observe-issue-terminal-label-over-pr-state"))
    t.eq(result.exit_code, 0)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.is_true(label_raise ~= nil)
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:blocked")
    t.is_true(has_value(label_raise.payload.remove_labels, "fkst-dev:reviewing"))
    t.eq(label_raise.payload.dedup_key, core._dedup_key({
      "reconcile",
      "label",
      proposal_id,
      "blocked",
      version,
    }))
    t.eq(find_raise(result.raises, "devloop_reviewing"), nil)
    t.eq(count_calls("--json title,body,comments,labels,state,updatedAt,assignees"), 1)
    t.eq(count_calls("--json headRefName,headRefOid,baseRefName,state,updatedAt,comments"), 1)
  end,

  test_observe_issue_reconciles_merged_terminal_label_from_marker = function()
    local proposal_id = "github-devloop/issue/owner/repo/42"
    local version = "ready/consensus-github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:merge-ready" }, "OPEN", {
      core.state_marker(proposal_id, "merged", version),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:merge-ready" } }), opts("observe-issue-terminal-label-reconcile-merged"))
    t.eq(result.exit_code, 0)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.is_true(label_raise ~= nil)
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:merged")
    t.is_true(has_value(label_raise.payload.remove_labels, "fkst-dev:merge-ready"))
    t.eq(label_raise.payload.source_ref.ref, "owner/repo#issue/42")
    t.eq(count_calls("--json title,body,comments,labels,state,updatedAt,assignees"), 1)
  end,

  test_observe_issue_missing_reviewing_label_does_not_change_pr_local_state = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    mock_issue_state({ "fkst-dev:enabled" }, "OPEN", {
      core.state_marker(event.proposal_id, "pr-open", ready_payload.dedup_key),
      core.pr_link_marker(event.proposal_id, 7, "devloop-owner-repo-42-01HY", ready_payload.dedup_key, "dev"),
    })
    mock_linked_pr_state({
      core.state_marker(event.proposal_id, "reviewing", ready_payload.dedup_key),
    })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled" } }), opts("observe-issue-pr-local-reviewing-no-label"))
    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "consensus.proposal"), nil)
    local label_raise = find_raise(result.raises, "github-proxy.github_issue_label_request")
    t.eq(label_raise.payload.add_labels[1], "fkst-dev:reviewing")
  end,

  test_observe_issue_linked_pr_fetch_failure_fails_closed = function()
    local event = reached()
    local ready_payload = core.build_devloop_ready_payload(event)
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:pr-open" }, "OPEN", {
      core.state_marker(event.proposal_id, "pr-open", ready_payload.dedup_key),
      core.pr_link_marker(event.proposal_id, 7, "devloop-owner-repo-42-01HY", ready_payload.dedup_key, "dev"),
    })
    mock_linked_pr_state({}, "OPEN", 1)

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:pr-open" } }), opts("observe-issue-pr-local-fetch-failure"))
    t.eq(result.exit_code, 1)
    t.eq(#result.raises, 0)
  end,

  test_observe_issue_blocked_decomposed_marker_reraises_missing_children = function()
    local event = decompose_event()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:blocked" }, "OPEN", {
      core.pr_link_marker(event.proposal_id, event.pr_number, "devloop-owner-repo-42-01HY", event.version, "dev"),
      core.state_marker(event.proposal_id, "blocked", event.version),
      merge_gate_fix_marker(event),
      core.decomposed_marker(event.proposal_id, event.version, event.pr_number, 3),
    })
    mock_linked_pr_state({})
    mock_decompose_child_issue_list(event, {})

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:blocked" } }), opts("observe-issue-decomposed-missing-children"))

    t.eq(result.exit_code, 0)
    local decompose = find_raise(result.raises, "devloop_decompose")
    t.eq(decompose.payload.schema, "github-devloop.decompose.v1")
    t.eq(decompose.payload.proposal_id, event.proposal_id)
    t.eq(decompose.payload.version, event.version)
    t.eq(decompose.payload.pr_number, event.pr_number)
    t.eq(decompose.payload.review_proposal_id, event.review_proposal_id)
    t.eq(decompose.payload.review_dedup_key, event.review_dedup_key)
    t.eq(decompose.payload.head_sha, event.head_sha)
    t.eq(decompose.payload.source_ref.ref, "owner/repo#pr/7")
  end,

  test_observe_issue_blocked_decomposed_marker_skips_when_children_complete = function()
    local event = decompose_event()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:blocked" }, "OPEN", {
      core.pr_link_marker(event.proposal_id, event.pr_number, "devloop-owner-repo-42-01HY", event.version, "dev"),
      core.state_marker(event.proposal_id, "blocked", event.version),
      merge_gate_fix_marker(event),
      core.decomposed_marker(event.proposal_id, event.version, event.pr_number, 3),
    })
    mock_linked_pr_state({})
    mock_decompose_child_issue_list(event, { 1, 2, 3 })

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:blocked" } }), opts("observe-issue-decomposed-complete-children"))

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_decompose"), nil)
  end,

  test_observe_issue_blocked_decomposed_marker_refuses_untrusted_marker = function()
    local event = decompose_event()
    mock_issue_state({ "fkst-dev:enabled", "fkst-dev:blocked" }, "OPEN", {
      core.pr_link_marker(event.proposal_id, event.pr_number, "devloop-owner-repo-42-01HY", event.version, "dev"),
      core.state_marker(event.proposal_id, "blocked", event.version),
      merge_gate_fix_marker(event),
      {
        body = core.decomposed_marker(event.proposal_id, event.version, event.pr_number, 3),
        author_login = "mallory",
      },
    })
    mock_linked_pr_state({})
    mock_decompose_child_issue_list(event, {})

    local result = run_observe(issue({ labels = { "fkst-dev:enabled", "fkst-dev:blocked" } }), opts("observe-issue-decomposed-forged"))

    t.eq(result.exit_code, 0)
    t.eq(find_raise(result.raises, "devloop_decompose"), nil)
  end,
}
