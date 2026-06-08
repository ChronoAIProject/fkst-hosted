local h = require("tests.devloop_core_helpers")
local core = h.core
local t = h.t

local proposal_id = "github-devloop/issue/owner/repo/42"
local base_version = "consensus:github-devloop/issue/owner/repo/42/2026-06-03T01-02-03Z"
local source_ref = {
  kind = "external",
  ref = "owner/repo#issue/42",
}

local function angles(extra)
  local values = {
    { angle = "structural", verdict = "abstain", reply = "Needs clearer boundaries.", digest = "Needs clearer boundaries." },
    { angle = "minimal", verdict = "approve", reply = "Small enough.", digest = "Small enough." },
  }
  for index, value in pairs(extra or {}) do
    values[index] = value
  end
  return values
end

local function fact(round, question, verdicts)
  return {
    round = round,
    question = question or "q-same",
    verdicts = verdicts or "v-same",
    dedup = base_version .. "/loop/" .. tostring(round),
  }
end

local function trusted(body)
  return {
    body = body,
    author_login = core.trusted_bot_login(),
  }
end

local function untrusted(body)
  return {
    body = body,
    author_login = "ordinary-user",
  }
end

return {
  test_converge_round_facts_ignore_non_bot_marker = function()
    local source_digest = core.source_ref_digest(source_ref)
    local marker = core.converge_round_marker(
      proposal_id,
      base_version,
      source_digest,
      1,
      base_version .. "/loop/1",
      "Which boundary should narrow?",
      angles()
    )

    local facts = core.converge_round_facts({ untrusted(marker) }, proposal_id, base_version, source_digest)
    t.eq(#facts, 0)
  end,

  test_is_true_stall_requires_three_identical_rounds = function()
    t.eq(core.is_true_stall({ fact(1) }, 1), false)
    t.eq(core.is_true_stall({ fact(1), fact(2) }, 2), false)
    t.eq(core.is_true_stall({ fact(1), fact(2), fact(3) }, 3), true)
  end,

  test_is_true_stall_requires_last_three_consecutive_rounds = function()
    local facts = {
      fact(1),
      fact(2),
      fact(4),
    }
    t.eq(core.is_true_stall(facts, 4), false)
  end,

  test_is_true_stall_false_when_round_three_question_changes = function()
    local facts = {
      fact(1),
      fact(2),
      fact(3, "q-different", "v-same"),
    }
    t.eq(core.is_true_stall(facts, 3), false)
  end,

  test_is_true_stall_false_when_round_three_verdicts_change = function()
    local facts = {
      fact(1),
      fact(2),
      fact(3, "q-same", "v-different"),
    }
    t.eq(core.is_true_stall(facts, 3), false)
  end,

  test_converge_marker_round_trips_and_digests_are_stable = function()
    local source_digest = core.source_ref_digest(source_ref)
    local consensus_dedup = base_version .. "/loop/3"
    local question = "  Which boundary\n\nshould   narrow?  "
    local angle_digests = angles()
    local first = core.converge_round_marker(
      proposal_id,
      core.converge_base_version(consensus_dedup),
      source_digest,
      3,
      consensus_dedup,
      question,
      angle_digests
    )
    local second = core.converge_round_marker(
      proposal_id,
      core.converge_base_version(consensus_dedup),
      source_digest,
      3,
      consensus_dedup,
      question,
      { angle_digests[2], angle_digests[1] }
    )
    t.eq(first, second)

    local facts = core.converge_round_facts({ trusted(first) }, proposal_id, base_version, source_digest)
    t.eq(#facts, 1)
    t.eq(facts[1].round, 3)
    t.eq(facts[1].dedup, consensus_dedup)
    t.eq(facts[1].question, core.converge_question_digest(question))
    t.eq(facts[1].verdicts, core.converge_verdicts_digest(angle_digests))
    t.eq(core.has_converge_round_marker({ trusted(first) }, proposal_id, base_version, source_digest, 3), true)
    t.eq(core.max_converge_round(facts), 3)
  end,

  test_converge_round_facts_keep_last_marker_for_same_round = function()
    local source_digest = core.source_ref_digest(source_ref)
    local first_question = "Which boundary should narrow first?"
    local last_question = "Which boundary should narrow last?"
    local first = core.converge_round_marker(
      proposal_id,
      base_version,
      source_digest,
      2,
      base_version .. "/loop/2",
      first_question,
      angles()
    )
    local last = core.converge_round_marker(
      proposal_id,
      base_version,
      source_digest,
      2,
      base_version .. "/loop/2",
      last_question,
      angles()
    )

    local facts = core.converge_round_facts({ trusted(first .. "\n" .. last) }, proposal_id, base_version, source_digest)
    t.eq(#facts, 1)
    t.eq(facts[1].round, 2)
    t.eq(facts[1].question, core.converge_question_digest(last_question))
  end,

  test_review_converge_facts_are_bound_to_issue_version_and_head = function()
    local source_digest = core.source_ref_digest({ kind = "external", ref = "owner/repo#pr/7" })
    local review_proposal_id = "github-devloop/pr-review/owner_repo/7/v1/abcdef1234567890"
    local issue_version = "ready/consensus-github-devloop/issue/owner/repo/42/v1"
    local head_sha = "abcdef1234567890"
    local marker = core.review_converge_round_marker(
      review_proposal_id,
      proposal_id,
      issue_version,
      head_sha,
      source_digest,
      2,
      "consensus:github-devloop/pr-review/owner_repo/7/v1/abcdef1234567890/loop/2",
      "Which review finding should narrow?",
      angles()
    )

    local mismatched_head = core.review_converge_round_facts(
      { trusted(marker) },
      review_proposal_id,
      proposal_id,
      issue_version,
      "fedcba0987654321",
      source_digest
    )
    local mismatched_version = core.review_converge_round_facts(
      { trusted(marker) },
      review_proposal_id,
      proposal_id,
      issue_version .. "/new",
      head_sha,
      source_digest
    )
    local matched = core.review_converge_round_facts(
      { trusted(marker) },
      review_proposal_id,
      proposal_id,
      issue_version,
      head_sha,
      source_digest
    )

    t.eq(#mismatched_head, 0)
    t.eq(#mismatched_version, 0)
    t.eq(#matched, 1)
    t.eq(matched[1].round, 2)
    t.eq(core.has_review_converge_round_marker({ trusted(marker) }, review_proposal_id, proposal_id, issue_version, head_sha, source_digest, 2), true)
  end,
}
