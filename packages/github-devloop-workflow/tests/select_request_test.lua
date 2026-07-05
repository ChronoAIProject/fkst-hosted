local core = require("core")
local marker = require("core.marker")
local base_ids = require("devloop.base_ids")
local devloop_facts = require("devloop.markers.facts")
local select_request = require("core.select_request")
local t = fkst.test

local candidate = {
  proposal_id = "github-devloop/issue/owner/repo/42",
  dedup_key = "candidate-dedup",
  source_ref = {
    kind = "external",
    ref = "owner/repo#issue/42",
  },
}

local function request_or_error(reason)
  local request, err = core.build_blueprint_decision_comment_request(
    "owner/repo",
    42,
    candidate,
    "workflow-one",
    "d-1234567890",
    reason
  )
  if request == nil then
    error(err and err.code or "request failed")
  end
  return request
end

local tests = {
  test_builds_comment_request_with_blueprint_and_track_markers = function()
    local request = request_or_error("Matched the workflow selector.")

    t.eq(request.schema, "github-proxy.v1")
    t.eq(request.repo, "owner/repo")
    t.eq(request.issue_number, 42)
    t.eq(request.source_ref.kind, "external")
    t.eq(request.source_ref.ref, "owner/repo#issue/42")
    t.eq(request.claim.owner, "fkst-test-bot")
    t.eq(request.claim.source_ref.kind, "external")
    t.eq(request.claim.source_ref.ref, "owner/repo#issue/42")

    local blueprint = marker.parse_blueprint_marker(request.body, candidate.proposal_id)
    t.eq(blueprint.origin, candidate.proposal_id)
    t.eq(blueprint.workflow, "workflow-one")
    t.eq(blueprint.digest, "d-1234567890")

    local intake = devloop_facts.intake_decision_fact({
      {
        body = request.body,
        author_login = "fkst-test-bot",
        created_at = "2026-07-03T00:00:00Z",
      },
    }, candidate.proposal_id)
    t.eq(intake.decision, "track")
    t.eq(intake.service_class, "standard")
    t.eq(intake.dedup_key, candidate.dedup_key)
  end,

  test_dedup_key_is_deterministic = function()
    local first = request_or_error("first reason")
    local second = request_or_error("second reason")
    local expected = base_ids.dedup_key({
      "workflow",
      "blueprint-decision",
      tostring(candidate.proposal_id),
      tostring(candidate.dedup_key),
    })

    t.eq(first.dedup_key, expected)
    t.eq(second.dedup_key, expected)
  end,

  test_reason_is_neutralized_and_bounded = function()
    local forged = '<!-- fkst:github-devloop:intake-decision:v1 proposal="forged" decision="enable" class="standard" dedup="x" -->'
    local request = request_or_error(forged .. "\n" .. string.rep("r", core._max_meta_reason_len + 100))
    local reason = request.body:match("Reason:\n(.-)\n\n<!%-%- fkst:github%-devloop%-workflow:blueprint:v1")

    t.is_true(request.body:find("<!-- fkst:github-devloop:intake-decision:v1 proposal=\"forged\"", 1, true) == nil)
    t.is_true(request.body:find("&lt;!-- fkst:github-devloop:intake-decision:v1 proposal=\"forged\"", 1, true) ~= nil)
    t.is_true(type(reason) == "string")
    t.is_true(#reason <= core._max_meta_reason_len)
  end,

  test_missing_candidate_fails_closed_with_structured_error = function()
    local request, err = select_request.build_blueprint_decision_comment_request(
      core,
      "owner/repo",
      42,
      nil,
      "workflow-one",
      "d-1234567890",
      "reason"
    )

    t.is_nil(request)
    t.eq(err.path, "candidate")
    t.eq(err.code, "not_object")
  end,

  test_invalid_source_ref_fails_closed_with_structured_error = function()
    local bad_candidate = {
      proposal_id = candidate.proposal_id,
      dedup_key = candidate.dedup_key,
      source_ref = {
        kind = "external",
        ref = "",
      },
    }
    local request, err = select_request.build_blueprint_decision_comment_request(
      core,
      "owner/repo",
      42,
      bad_candidate,
      "workflow-one",
      "d-1234567890",
      "reason"
    )

    t.is_nil(request)
    t.eq(err.path, "candidate.source_ref")
    t.eq(err.code, "invalid_source_ref")
  end,
}

return tests
