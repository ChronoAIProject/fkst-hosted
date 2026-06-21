local core = require("core")
local saga = require("std.saga")

local spec = {
  consumes = { "devloop_intake_tick" },
  produces = {
    "devloop_intake_candidate",
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_create_request",
  },
  fanout = { "devloop_intake_tick" },
  stall_window = "30s",
}

local INTAKE_LIMIT = 100

local function raise_reintake_refusal(repo, issue_number, proposal_id, command, reason)
  local source_ref = {
    kind = "external",
    ref = tostring(repo) .. "#issue/" .. tostring(issue_number),
  }
  local request = core.build_operator_issue_command_refusal_request(
    repo,
    tostring(issue_number),
    command,
    reason,
    source_ref
  )
  core.log_cas_decision("intake_scan", proposal_id, { state = nil, version = nil }, "reintake-command", "candidate", "refused(" .. tostring(reason) .. ")", "operator reintake precondition failed")
  core.log_raise("intake_scan", proposal_id, "github-proxy.github_issue_comment_request", request)
end

local function handle_pending_reintake(repo, issue, current, proposal_id)
  local command = core.pending_reintake_command(current.comments)
  if command == nil then
    return false
  end
  if current.state ~= "OPEN" then
    raise_reintake_refusal(repo, issue.number, proposal_id, command, "reintake requires an open issue")
    return true
  end
  if not core.has_intake_decision_marker(current.comments, proposal_id) then
    raise_reintake_refusal(repo, issue.number, proposal_id, command, "reintake requires an existing intake decision")
    return true
  end
  if core.is_intake_held(current.labels) then
    core.log_cas_decision("intake_scan", proposal_id, { state = nil, version = nil }, "reintake-command", "candidate", "skip-held", "fkst-dev:hold label is present")
    return true
  end
  if core.should_skip_known_intake_issue(current.labels) then
    raise_reintake_refusal(repo, issue.number, proposal_id, command, "reintake requires no active devloop state")
    return true
  end
  if not core.claim_issue_for_management("intake_scan", repo, issue.number, current, proposal_id) then
    return true
  end
  local payload = core.build_intake_scan_candidate(repo, issue, command, now())
  core.log_apply("intake_scan", proposal_id, nil, nil, { add = {}, remove = {} }, {
    "devloop_intake_candidate",
  })
  core.log_raise("intake_scan", proposal_id, "devloop_intake_candidate", payload)
  return true
end

local function intake_scan_done(_event)
  return false
end

local function act_intake_scan(event)
  core.log_entry("intake_scan", event, "github-devloop/intake", "tick")
  core.assert_trusted_bot_configured()

  local repo = core.read_intake_repo()
  if repo == nil then
    core.log_cas_decision("intake_scan", "github-devloop/intake", { state = nil, version = nil }, "tick", "candidate", "skip-invalid-repo", "FKST_GITHUB_REPO is missing or invalid")
    return
  end

  local list = core.fetch_shared_issue_intake_list(repo, INTAKE_LIMIT, {
    timeout = 30,
    poll_key = core.entity_list_poll_key(event),
  })
  if list.exit_code ~= 0 then
    error("github-devloop: gh issue intake list failed: " .. tostring(list.stderr))
  end

  for _, issue in ipairs(core.parse_issue_list_intake(list.stdout, INTAKE_LIMIT)) do
    local issue_number = tostring(issue.number or "")
    if core.issue_ref_round_trips(repo, issue_number) then
      local proposal_id = core.proposal_id(repo, issue_number)
      -- Stale-tolerant scan read: intake_scan only screens candidates and
      -- re-checks every poll, so a bounded-stale view is safe; the downstream
      -- intake_judge re-reads fresh before any CAS decision. Cache collapses the
      -- per-poll re-read of the same issue (a dominant GraphQL drain).
      local view = core.gh_exec_cached(
        function()
          return core.gh_issue_view_intake_scan(repo, issue_number, 30)
        end,
        core.gh_read_cache_key("intake-scan", repo, issue_number),
        90
      )
      if view.exit_code ~= 0 then
        error("github-devloop: gh issue intake scan view failed: " .. tostring(view.stderr))
      end
      local current = core.parse_issue_view_intake_scan(view.stdout)
      current.updated_at = current.updated_at or issue.updated_at
      core.log_forged_markers("intake_scan", proposal_id, current.comments)
      if not handle_pending_reintake(repo, issue, current, proposal_id)
        and current.state == "OPEN"
        and not core.should_skip_known_intake_issue(current.labels)
        and not core.has_intake_decision_marker(current.comments, proposal_id)
        and core.claim_issue_for_management("intake_scan", repo, issue_number, current, proposal_id) then
        -- We just wrote the assignee claim; drop the scan-cache slot so the next
        -- poll re-reads the fresh post-claim view (now self-assigned -> held, no
        -- redundant re-claim write) instead of re-screening the stale pre-claim
        -- view for up to the TTL.
        cache_set(core.gh_read_cache_key("intake-scan", repo, issue_number), "")
        local payload = core.build_intake_scan_candidate(repo, issue, nil, now())
        core.log_apply("intake_scan", proposal_id, nil, nil, { add = {}, remove = {} }, {
          "devloop_intake_candidate",
        })
        core.log_raise("intake_scan", proposal_id, "devloop_intake_candidate", payload)
      end
    end
  end
end

return saga.department(spec, {
  done = intake_scan_done,
  act = act_intake_scan,
  wrap = core.wrap_pipeline_failure,
  name = "intake_scan",
})
