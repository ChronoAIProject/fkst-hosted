local core = require("core")
local mapping = require("departments.reply.mapping")
local saga = require("workflow.saga")

local spec = {
  consumes = { "consensus.consensus_reached" },
  produces = { "reply" },
  fanout = { "consensus.consensus_reached" },
  stall_window = "30s",
}

local function classify(payload)
  if type(payload) ~= "table"
    or payload.schema ~= "consensus.consensus_reached.v1"
    or type(payload.proposal_id) ~= "string"
    or payload.proposal_id:match("^autochrono/issue/") == nil then
    return "foreign"
  end

  local repo, issue_number = core.parse_proposal_id(payload.proposal_id)
  if repo == nil or not core.issue_ref_round_trips(repo, issue_number) then
    error("autochrono: consensus-result-invalid: owned proposal_id is malformed")
  end
  local expected_source_ref = tostring(repo) .. "#issue/" .. tostring(issue_number)
  if (payload.decision ~= "approve" and payload.decision ~= "reject")
    or not core.validate_reached(payload)
    or payload.source_ref.kind ~= "external"
    or payload.source_ref.ref ~= expected_source_ref then
    error("autochrono: consensus-result-invalid: owned consensus result violates the consumer contract")
  end
  if payload.decision == "reject" then
    return "ignored"
  end
  return "route", repo, issue_number
end

local function reply_done(event)
  local payload = event.payload
  local disposition, repo, issue_number = classify(payload)
  if disposition ~= "route" then
    return true
  end

  local cache_key = core.replied_cache_key(repo, issue_number)
  local already_replied = false
  with_lock(cache_key, function()
    already_replied = cache_get(cache_key) ~= nil
  end)
  return already_replied
end

local function act_reply(event)
  local payload = event.payload
  local disposition, repo, issue_number = classify(payload)
  if disposition ~= "route" then
    return
  end

  local cache_key = core.replied_cache_key(repo, issue_number)
  with_lock(cache_key, function()
    if cache_get(cache_key) then
      return
    end

    raise("reply", mapping.build_reply(payload, repo, issue_number))
    cache_set(cache_key, core.reply_dedup_key(repo, issue_number))
  end)
end

return saga.department(spec, {
  done = reply_done,
  act = act_reply,
  wrap = core.wrap_pipeline_failure,
  name = "reply",
})
