local core = require("core")
local t = fkst.test

local function proposal(extra)
  local value = {
    schema = "consensus.proposal.v1",
    proposal_id = "proposal-42",
    title = "Judge a repository change",
    body = "Judge the supplied change against the repository.",
    dedup_key = "proposal-42-v1",
    source_ref = {
      kind = "proposal",
      ref = "demo/consensus/42",
    },
  }
  for key, field in pairs(extra or {}) do
    value[key] = field
  end
  return value
end

local function result(angle, verdict)
  return {
    angle = angle,
    verdict = verdict,
    reply = angle .. " reply",
    stdout = angle .. " output",
    exit_code = 0,
  }
end

local function prompts(value)
  local p1 = {
    result("teleology", "approve"),
    result("parsimony", "abstain"),
  }
  return {
    core.build_angle_prompt(value, "teleology"),
    core.build_rebuttal_prompt(value, p1[1], { p1[2] }),
    core.build_synthesis_prompt(value, p1, p1),
  }
end

return {
  test_optional_worktree_accepts_dot_or_absolute_single_line_paths = function()
    t.eq(core.is_eligible(proposal()), true)
    t.eq(core.is_eligible(proposal({ worktree = "." })), true)
    t.eq(core.is_eligible(proposal({ worktree = "/tmp/judged repo" })), true)
    t.eq(core.is_eligible(proposal({ worktree = "relative/repo" })), false)
    t.eq(core.is_eligible(proposal({ worktree = "/tmp/repo\nother" })), false)
    t.eq(core.is_eligible(proposal({ worktree = "/" .. string.rep("x", 4096) })), false)
    t.eq(core.is_eligible(proposal({ worktree = false })), false)
  end,

  test_prompt_boundary_matches_resolved_worktree = function()
    for _, prompt in ipairs(prompts(proposal())) do
      t.is_true(prompt:find("You are running in an empty runtime scratch directory, not a repository checkout.", 1, true) ~= nil)
      t.is_true(prompt:find("Do not clone, checkout, fetch with git, create branches, or modify any repository.", 1, true) ~= nil)
      t.is_true(prompt:find("Read required source content only from the context manifest below.", 1, true) ~= nil)
      t.is_nil(prompt:find("read-only checkout of the judged repository", 1, true))
    end

    for _, prompt in ipairs(prompts(proposal({ worktree = "/tmp/judged-repo" }))) do
      t.is_true(prompt:find("You are running in a read-only checkout of the judged repository.", 1, true) ~= nil)
      t.is_true(prompt:find("The context bundle below remains the pinned snapshot of record.", 1, true) ~= nil)
      t.is_true(prompt:find("You may read any file in this checkout as additional evidence.", 1, true) ~= nil)
      t.is_true(prompt:find("When a claim about repository source is load-bearing, cite path:line.", 1, true) ~= nil)
      t.is_true(prompt:find("Do not clone, fetch, checkout, create branches, or modify anything.", 1, true) ~= nil)
      t.is_nil(prompt:find("empty runtime scratch directory", 1, true))
    end
  end,
}
