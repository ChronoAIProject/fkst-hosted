local requests_labels = require("devloop.requests.labels")
local requests_lifecycle = require("devloop.requests.lifecycle")
local base_ids = require("devloop.base_ids")
local context_bundle = require("devloop.context_bundle")

local payloads_builders = require("devloop.payloads.builders")
local v_validate_proposal = require("devloop.validators.validate_proposal")
local E = {}

local service_classes = {
  expedite = true,
  standard = true,
  background = true,
}

function E.is_execution_service_class(value)
  return service_classes[tostring(value or "")] == true
end

function E.normalize_execution_service_class(value)
  local text = tostring(value or ""):lower()
  if service_classes[text] then
    return text
  end
  return "standard"
end

function E.build_execution_request_payload(source)
  local payload = {
    schema = "github-devloop.execution-request.v1",
    proposal_id = source.proposal_id,
    dedup_key = source.dedup_key,
    source_ref = base_ids.normalize_source_ref(source.source_ref),
  }
  if type(source.origin) == "table" then
    payload.origin = source.origin
  end
  if source.service_class ~= nil then
    payload.service_class = E.normalize_execution_service_class(source.service_class)
  end
  if source.framing ~= nil then
    payload.framing = tostring(source.framing)
  end
  if type(source.context) == "table" then
    payload.context = source.context
  end
  return payload
end

function E.execution_intake_hand_off(request)
  return {
    kind = "own-intake-decision",
    proposal_id = request.proposal_id,
    decision = "enable",
    dedup_key = request.dedup_key,
    source_ref = base_ids.normalize_source_ref(request.source_ref),
  }
end

local function workflow_origin_issue(request)
  local origin = type(request) == "table" and request.origin or nil
  local lineage = type(origin) == "table" and origin.lineage or nil
  if type(origin) ~= "table"
    or origin.package ~= "github-devloop-workflow"
    or origin.route ~= "workflow-child"
    or origin.decision ~= "committed-child"
    or type(lineage) ~= "table"
    or type(lineage.origin) ~= "string"
    or type(lineage.blueprint_digest) ~= "string"
    or lineage.blueprint_digest == ""
    or type(lineage.slot) ~= "string"
    or lineage.slot == "" then
    return nil
  end

  local repo, issue_number = base_ids.parse_proposal_id(lineage.origin)
  if repo == nil
    or issue_number == nil
    or base_ids.proposal_id(repo, issue_number) ~= lineage.origin
    or lineage.origin == request.proposal_id then
    return nil
  end
  return {
    repo = repo,
    issue_number = issue_number,
  }
end

function E.build_execution_start_proposal(core, repo, issue_number, request, current, event_ts, dept)
  local origin_issue = workflow_origin_issue(request)
  local issue = {
    repo = repo,
    number = issue_number,
    title = current.title,
    updated_at = current.updated_at,
    source_ref = request.source_ref,
    content_fetch = context_bundle.context_fetch_ref_from_bundle(core, {
      dept = dept or "execute_start",
      repo = repo,
      issue_number = issue_number,
      proposal_id = request.proposal_id,
      version = request.dedup_key,
      tick = event_ts,
      origin_issue_repo = origin_issue and origin_issue.repo or nil,
      origin_issue_number = origin_issue and origin_issue.issue_number or nil,
    }),
  }
  local proposal = payloads_builders.build_board_proposal(core, issue, event_ts)
  proposal.dedup_key = request.dedup_key
  proposal.effect_version = request.dedup_key
  proposal.intake_hand_off = E.execution_intake_hand_off(request)
  return v_validate_proposal.validate_proposal(proposal) and proposal or nil
end

E.workflow_origin_issue = workflow_origin_issue

function E.build_execution_start_effects(core, repo, issue_number, request, current, event_ts, dept)
  local proposal = E.build_execution_start_proposal(core, repo, issue_number, request, current, event_ts, dept)
  if proposal == nil then
    return nil
  end
  local issue_ref = {
    repo = repo,
    number = issue_number,
    source_ref = request.source_ref,
  }
  local origin_issue = workflow_origin_issue(request)
  local dependency_origin = origin_issue
    and base_ids.proposal_id(origin_issue.repo, origin_issue.issue_number)
    or nil
  return {
    proposal = proposal,
    thinking_comment_request = requests_lifecycle.build_observe_comment_request(
      core,
      issue_ref,
      proposal,
      dependency_origin
    ),
    thinking_label_request = requests_labels.build_thinking_label_request(issue_ref, proposal),
  }
end

return E
