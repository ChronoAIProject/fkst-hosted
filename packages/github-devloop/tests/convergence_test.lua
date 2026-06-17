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
    t.eq(facts[1].narrowed_question, "Which boundary should narrow?")
    t.eq(facts[1].angle_digests[1].digest, "Small enough.")
    t.eq(core.has_converge_round_marker({ trusted(first) }, proposal_id, base_version, source_digest, 3), true)
    t.eq(core.max_converge_round(facts), 3)
  end,

  test_converge_marker_replay_fields_escape_delimiters = function()
    local source_digest = core.source_ref_digest(source_ref)
    local marker = core.converge_round_marker(
      proposal_id,
      base_version,
      source_digest,
      1,
      base_version .. "/loop/1",
      "Which boundary should narrow?",
      {
        { angle = "minimal", verdict = "abstain", digest = "contains | pipe; semicolon % percent" },
      }
    )

    local facts = core.converge_round_facts({ trusted(marker) }, proposal_id, base_version, source_digest)
    t.eq(#facts, 1)
    t.eq(facts[1].angle_digests[1].digest, "contains | pipe; semicolon % percent")
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

  test_append_converge_round_fact_preserves_existing_facts_and_appends_digest_fact = function()
    local existing = {
      fact(1, "q-one", "v-one"),
    }
    local question = "  Which boundary\nshould narrow next?  "
    local angle_digests = angles()
    local appended = core.append_converge_round_fact(
      existing,
      2,
      question,
      angle_digests,
      base_version .. "/loop/2"
    )

    t.eq(#appended, 2)
    t.eq(appended[1], existing[1])
    t.eq(appended[2].round, 2)
    t.eq(appended[2].question, core.converge_question_digest(question))
    t.eq(appended[2].verdicts, core.converge_verdicts_digest(angle_digests))
    t.eq(appended[2].dedup, base_version .. "/loop/2")
  end,

  test_converge_budget_round_counts_proposal_across_drift = function()
    local source_a = core.source_ref_digest(source_ref)
    local source_b = core.source_ref_digest({ kind = "external", ref = "owner/repo#issue/42?loop=8" })
    local drift_version = base_version .. "/drifted"
    local boundary_question = "Same boundary"
    local boundary_angles = angles()
    local comments = {
      trusted(core.converge_round_marker(proposal_id, base_version, source_a, 6, base_version .. "/loop/6", boundary_question, boundary_angles)),
      trusted(core.converge_round_marker(proposal_id, drift_version, source_b, 8, drift_version .. "/loop/8", boundary_question, boundary_angles)),
      trusted(core.converge_round_marker("github-devloop/issue/owner/repo/99", drift_version, source_b, 13, drift_version .. "/loop/13", "Other", angles())),
    }
    local filtered = core.converge_round_facts(comments, proposal_id, base_version, source_a)
    t.eq(core.max_converge_round(filtered), 6)
    t.eq(core.converge_budget_round(comments, proposal_id), 8)
    t.eq(core.converge_boundary_budget_round(comments, proposal_id, boundary_question, boundary_angles), 8)
  end,

  test_converge_boundary_budget_round_ignores_changed_question_verdict_boundary = function()
    local source_a = core.source_ref_digest(source_ref)
    local source_b = core.source_ref_digest({ kind = "external", ref = "owner/repo#issue/42?loop=8" })
    local drift_version = base_version .. "/drifted"
    local boundary_question = "Current boundary"
    local boundary_angles = angles()
    local comments = {
      trusted(core.converge_round_marker(proposal_id, base_version, source_a, 6, base_version .. "/loop/6", boundary_question, boundary_angles)),
      trusted(core.converge_round_marker(proposal_id, drift_version, source_b, 8, drift_version .. "/loop/8", "Different boundary", {
        { angle = "minimal", verdict = "approve", digest = "different" },
      })),
    }
    t.eq(core.converge_budget_round(comments, proposal_id), 8)
    t.eq(core.converge_boundary_budget_round(comments, proposal_id, boundary_question, boundary_angles), 6)
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

  test_review_converge_budget_round_counts_review_saga_across_drift = function()
    local source_a = core.source_ref_digest({ kind = "external", ref = "owner/repo#pr/7" })
    local source_b = core.source_ref_digest({ kind = "external", ref = "owner/repo#pr/7?loop=8" })
    local review_proposal_id = "github-devloop/pr-review/owner_repo/7/v1/abcdef1234567890"
    local issue_version = "ready/consensus-github-devloop/issue/owner/repo/42/v1"
    local drift_version = issue_version .. "/drifted"
    local head_sha = "abcdef1234567890"
    local drift_head = "fedcba0987654321"
    local comments = {
      trusted(core.review_converge_round_marker(review_proposal_id, proposal_id, issue_version, head_sha, source_a, 6, "review/loop/6", "Review 6", angles())),
      trusted(core.review_converge_round_marker(review_proposal_id, proposal_id, drift_version, drift_head, source_b, 8, "review/loop/8", "Review 8", angles())),
      trusted(core.review_converge_round_marker(review_proposal_id, "github-devloop/issue/owner/repo/99", drift_version, drift_head, source_b, 13, "review/loop/13", "Other", angles())),
    }
    local filtered = core.review_converge_round_facts(comments, review_proposal_id, proposal_id, issue_version, head_sha, source_a)
    t.eq(core.max_converge_round(filtered), 6)
    t.eq(core.review_converge_budget_round(comments, review_proposal_id, proposal_id), 8)
  end,
}
