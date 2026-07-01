local devloop_base = require("devloop.base")
local base_ids = require("devloop.base_ids")
local m_claims = require("devloop.claims")
local parsers_issue = require("devloop.parsers.issue")
local core = require("core")
local operator_commands = require("devloop.operator_commands")
local queue = require("devloop.queue")
local saga = require("workflow.saga")
local m_facts = require("devloop.markers.facts")

local spec = {
  consumes = { "github-proxy.github_entity_changed" },
  produces = {
    "devloop_intake_candidate",
    "github-proxy.github_issue_comment_request",
    "github-proxy.github_issue_create_request",
  },
  fanout = { "github-proxy.github_entity_changed" },
  stall_window = "30s",
}

local function raise_reintake_refusal(repo, issue_number, proposal_id, command, reason, source_ref)
  local request = operator_commands.build_operator_issue_command_refusal_request(
    core,
    repo,
    tostring(issue_number),
    command,
    reason,
    source_ref
  )
  core.log_cas_decision("admission", proposal_id, { state = nil, version = nil }, "reintake-command", "candidate", "refused(" .. tostring(reason) .. ")", "operator reintake precondition failed")
  core.log_raise("admission", proposal_id, "github-proxy.github_issue_comment_request", request)
end

local function handle_pending_reintake(repo, issue, current, proposal_id, source_ref)
  local command = core.pending_reintake_command(current.comments)
  if command == nil then
    return false
  end
  if current.state ~= "OPEN" then
    raise_reintake_refusal(repo, issue.number, proposal_id, command, "reintake requires an open issue", source_ref)
    return true
  end
  if not m_facts.has_intake_decision_marker(core, current.comments, proposal_id) then
    raise_reintake_refusal(repo, issue.number, proposal_id, command, "reintake requires an existing intake decision", source_ref)
    return true
  end
  if devloop_base.is_intake_held(current.labels) then
    core.log_cas_decision("admission", proposal_id, { state = nil, version = nil }, "reintake-command", "candidate", "skip-held", "fkst-dev:hold label is present")
    return true
  end
  if core.should_skip_known_intake_issue(current.labels) then
    raise_reintake_refusal(repo, issue.number, proposal_id, command, "reintake requires no active devloop state", source_ref)
    return true
  end
  if not m_claims.claim_issue_for_management(core, "admission", repo, issue.number, current, proposal_id) then
    return true
  end
  local payload = core.build_intake_admission_candidate(repo, issue, command, now())
  core.log_apply("admission", proposal_id, nil, nil, { add = {}, remove = {} }, {
    "devloop_intake_candidate",
  })
  core.log_raise("admission", proposal_id, "devloop_intake_candidate", payload)
  return true
end

local function done(_event)
  return false
end

local function admit_issue_event(event, entity)
  entity = entity or event.payload or {}
  core.log_entry("admission", event, "github-devloop/intake", core.payload_field(entity, "dedup_key"))
  local repo, issue_number = devloop_base.parse_issue_source_ref(entity.source_ref)
  if repo == nil or issue_number == nil then
    core.log_cas_decision("admission", "unknown", { state = nil, version = nil }, "entity", "candidate", "skip-foreign(source_ref)", "invalid issue source_ref")
    return
  end
  local proposal_id = base_ids.proposal_id(repo, issue_number)
  devloop_base.assert_trusted_bot_configured()

  local view = core.gh_issue_view(repo, issue_number, "title,body,createdAt,updatedAt,labels,comments,state,assignees,author", 30)
  if view.exit_code ~= 0 then
    error("github-devloop-intake: gh-issue-admission-view-failed: gh issue admission view failed: " .. tostring(view.stderr))
  end
  local current = parsers_issue.parse_issue_view_intake_judge(core, view.stdout)
  current.updated_at = current.updated_at or entity.updated_at
  current.number = issue_number

  core.log_forged_markers("admission", proposal_id, current.comments)
  local issue = {
    number = issue_number,
    title = current.title,
    body = current.body,
    updated_at = current.updated_at,
  }

  if handle_pending_reintake(repo, issue, current, proposal_id, entity.source_ref) then
    return
  end
  if current.state ~= "OPEN" then
    core.log_cas_decision("admission", proposal_id, { state = nil, version = nil }, "entity", "candidate", "skip-closed", "fresh issue is not open")
    return
  end
  if core.should_skip_known_intake_issue(current.labels) then
    core.log_cas_decision("admission", proposal_id, { state = nil, version = nil }, "entity", "candidate", "skip-known-state", "fresh issue labels show an active devloop state")
    return
  end
  if m_facts.has_intake_decision_marker(core, current.comments, proposal_id) then
    core.log_cas_decision("admission", proposal_id, { state = nil, version = nil }, "entity", "candidate", "skip-intake-decision", "trusted intake decision marker is already visible")
    return
  end
  if not m_claims.claim_issue_for_management(core, "admission", repo, issue_number, current, proposal_id) then
    return
  end

  local payload = core.build_intake_admission_candidate(repo, issue, nil, now())
  core.log_apply("admission", proposal_id, nil, nil, { add = {}, remove = {} }, {
    "devloop_intake_candidate",
  })
  core.log_raise("admission", proposal_id, "devloop_intake_candidate", payload)
end

local function act_entity_changed(event)
  local entity = event.payload or {}
  if entity.type ~= "issue" then
    return
  end
  if tostring(entity.state or ""):upper() ~= "OPEN" then
    return
  end
  admit_issue_event(event, entity)
end

local handlers = {
  ["github-proxy.github_entity_changed"] = act_entity_changed,
}

local function act(event)
  local handled = queue.dispatch_consumed_queue("admission", spec, event, handlers, "github-devloop-intake")
  if not handled then
    error("github-devloop-intake: consumed-queue-unrouted: " .. tostring(event and event.queue or ""))
  end
end

return saga.department(spec, {
  done = done,
  act = act,
  wrap = core.wrap_pipeline_failure,
  name = "admission",
})
