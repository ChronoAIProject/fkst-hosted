local core = require("core")
local t = fkst.test
local action_label = "⟦FKST:ACTION⟧"
local reason_label = "⟦FKST:REASON⟧"

local function has_value(values, expected)
  for _, value in ipairs(values or {}) do
    if value == expected then
      return true
    end
  end
  return false
end

local function source_ref()
  return {
    kind = "external",
    ref = "owner/repo#issue/42",
  }
end

local function issue(extra)
  local value = {
    schema = "github-proxy.v1",
    type = "issue",
    repo = "owner/repo",
    number = 42,
    title = "Implement decision recorder",
    url = "https://github.example/owner/repo/issues/42",
    state = "OPEN",
    updated_at = "2026-06-03T01:02:03Z",
    labels = { "fkst-dev:enabled" },
    dedup_key = "owner/repo#issue#42@2026-06-03T01:02:03Z",
    source_ref = source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function reached(extra)
  local value = {
    schema = "consensus.consensus_reached.v1",
    proposal_id = "github-devloop/issue/owner/repo/42",
    decision = "approve",
    body = "All angles approve.",
    dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    source_ref = source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function unresolved(extra)
  local value = {
    schema = "consensus.consensus_unresolved.v1",
    proposal_id = "github-devloop/issue/owner/repo/42",
    dedup_key = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z",
    source_ref = source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function stuck(extra)
  local value = {
    schema = "github-devloop.stuck.v1",
    proposal_id = "github-devloop/issue/owner/repo/42",
    dedup_key = "github-devloop/issue/owner/repo/42/stuck/3/consensus-github-devloop/issue/owner/repo/42/v1",
    source_ref = source_ref(),
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function meta_answer(action, reason)
  return action_label .. " " .. action .. "\n" .. reason_label .. " " .. reason
end


return {
  core = core,
  t = t,
  action_label = action_label,
  reason_label = reason_label,
  has_value = has_value,
  source_ref = source_ref,
  issue = issue,
  reached = reached,
  unresolved = unresolved,
  stuck = stuck,
  meta_answer = meta_answer,
}
