local S = {}

function S.install(M)

local source_ref_derivations = {
  entity = true,
  issue = true,
  pr = true,
}

local marker_fields = {
  state = { proposal = true, state = true, version = true, stage_rank = true, effects = true },
  ["converge-round"] = {
    proposal = true,
    version = true,
    source_ref = true,
    round = true,
    question = true,
    verdicts = true,
    dedup = true,
    narrowed_question = true,
    angle_digests = true,
  },
  ["review-converge-round"] = {
    proposal = true,
    issue_proposal = true,
    version = true,
    head_sha = true,
    source_ref = true,
    round = true,
    question = true,
    verdicts = true,
    dedup = true,
    narrowed_question = true,
    angle_digests = true,
  },
  ["dependency-release"] = { proposal = true, version = true },
  implementing = { proposal = true, dedup = true, branch = true, head_sha = true, base_branch = true, base_sha = true },
  ["pr-link"] = { proposal = true, pr = true, branch = true, impl_version = true, base_branch = true },
  ["review-result"] = {
    proposal = true,
    issue_proposal = true,
    decision = true,
    dedup = true,
    fix_round = true,
    gap = true,
  },
  ["review-meta"] = { proposal = true, dedup = true, action = true, version = true, gap = true, reason = true },
  ["fix-reflection"] = { proposal = true, dedup = true, verdict = true, version = true, fix_round = true },
  ["merge-gate"] = {
    proposal = true,
    pr = true,
    version = true,
    review_proposal = true,
    review_dedup = true,
    head_sha = true,
    gate_baseline_sha = true,
    reason = true,
  },
  ["merge-ready"] = {
    proposal = true,
    pr = true,
    version = true,
    review_proposal = true,
    review_dedup = true,
    head_sha = true,
  },
  ["review-carry-over"] = {
    proposal = true,
    version = true,
    old_review_proposal = true,
    old_review_dedup = true,
    approved_head_sha = true,
    new_review_proposal = true,
    new_review_dedup = true,
    new_head_sha = true,
    base_head_sha = true,
    proof = true,
  },
  ["impl-failure"] = { proposal = true, reason = true, attempt = true, dedup = true },
  merging = { proposal = true, pr = true, version = true, head_sha = true },
  decomposed = { proposal = true, version = true, pr = true, count = true },
}

local required_replay_payload_fields = {
  fixing = {
    gate_baseline_sha = "build_replayed_fixing_payload copies merge-gate.gate_baseline_sha",
  },
}

local function fact(family, freshness)
  return { family = family, freshness = freshness }
end

local function effect(kinds, completeness, completeness_derivation)
  local declared_kinds = kinds
  if type(kinds) ~= "table" then
    declared_kinds = {}
    for index = 1, tonumber(kinds) or 0 do
      declared_kinds[index] = "effect-" .. tostring(index)
    end
  end
  return {
    intent_count = #declared_kinds,
    kinds = declared_kinds,
    completeness = completeness,
    completeness_derivation = completeness_derivation,
  }
end

local transition_table = {
  {
    from_state = "thinking",
    to_states = { "ready", "blocked" },
    driving_queue = "consensus.proposal",
    payload_builder = M.build_proposal,
    dedup_shape = "proposal:<proposal_id>/<updated_at> or consensus:<base_version>/loop/<n>",
    required_facts = { fact("state", "marker-read"), fact("converge-round", "marker-read") },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      dedup_key = "marker:state.version",
      source_ref = "source_ref:issue",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect({ "consensus.proposal" }, "consensus proposal dedup is derived from state.version or next complete converge-round"),
    marker_facts = "state:v1 thinking plus optional converge-round:v1",
    kickoff = "consensus.proposal",
    replay = "Initial thinking reuses the state version as proposal dedup; convergence replays the next /loop/N from the latest complete converge-round marker.",
  },
  {
    from_state = "ready",
    to_states = { "implementing" },
    driving_queue = "devloop_ready",
    payload_builder = M.build_devloop_ready_payload,
    dedup_shape = "ready/<state.version>",
    required_facts = { fact("state", "marker-read"), fact("dependency-release", "marker-read") },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      dedup_key = "marker:state.version",
      source_ref = "source_ref:issue",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect(
      { "result-marker", "ready-label", "devloop_ready" },
      "ready replay is complete only when the result marker and ready label are visible, and observe_issue can re-raise devloop_ready while still ready",
      "result_effects_complete"
    ),
    marker_facts = "state:v1 ready",
    kickoff = "devloop_ready",
    replay = "Raise ready/<version> after dependency gate re-derives satisfied blockers.",
  },
  {
    from_state = "implementing",
    to_states = { "pr-open", "impl-failed" },
    driving_queue = "github-proxy.github_entity_changed",
    payload_builder = M.build_devloop_open_pr_payload,
    dedup_shape = "open-pr-kickoff/<proposal_id>/<impl_version>/<branch>",
    required_facts = {
      fact("state", "marker-read"),
      fact("implementing", "marker-read"),
      fact("branch-head", "fetch-before-compare"),
    },
    payload_fields = {
      proposal_id = "marker:implementing.proposal",
      version = "marker:implementing.dedup",
      branch = "marker:implementing.branch",
      head_sha = "marker:implementing.head_sha",
      base_branch = "marker:implementing.base_branch",
      source_ref = "source_ref:issue",
    },
    version_identity = "implementing.dedup",
    effects = effect({ "github-proxy.github_entity_changed" }, "open-pr payload is complete when implementing marker and fetched branch head agree"),
    marker_facts = "state:v1 implementing plus implementing:v1",
    kickoff = "github-proxy.github_entity_changed",
    replay = "Branch poll re-derives PR open or impl-failed from branch/worktree facts.",
  },
  {
    from_state = "impl-failed",
    to_states = { "implementing" },
    driving_queue = "devloop_ready",
    payload_builder = M.build_devloop_ready_payload,
    dedup_shape = "ready/<state.version> with impl_retry_attempt=<impl-failure.attempt+1>",
    required_facts = { fact("state", "marker-read"), fact("impl-failure", "marker-read"), fact("dependency-release", "marker-read") },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      dedup_key = "marker:impl-failure.dedup",
      source_ref = "source_ref:issue",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect({ "devloop_ready" }, "impl-failed replay is complete when trusted codex-failed impl-failure attempt is below the retry ceiling"),
    marker_facts = "state:v1 impl-failed plus impl-failure:v1 reason=codex-failed attempt<N",
    kickoff = "devloop_ready",
    replay = "Observe re-raises ready/<version> after one observe tick for bounded codex-failed implementation retries.",
  },
  {
    from_state = "pr-open",
    to_states = { "reviewing" },
    driving_queue = "devloop_reviewing",
    payload_builder = M.build_devloop_reviewing_payload,
    dedup_shape = "reviewing/<proposal_id>/<impl_version>/<pr>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
    },
    payload_fields = {
      proposal_id = "marker:pr-link.proposal",
      pr_number = "marker:pr-link.pr",
      version = "marker:pr-link.impl_version",
      source_ref = "source_ref:pr",
    },
    version_identity = "pr-link.impl_version",
    effects = effect({ "devloop_reviewing" }, "reviewing payload is complete when linked open PR head/base still match the pr-link marker"),
    marker_facts = "state:v1 pr-open plus pr-link:v1",
    kickoff = "devloop_reviewing",
    replay = "Observe re-fetches the linked PR and raises review for the linked PR head.",
  },
  {
    from_state = "reviewing",
    to_states = { "merge-ready", "fixing", "review-meta", "blocked" },
    driving_queue = "devloop_reviewing",
    payload_builder = M.build_devloop_reviewing_payload,
    dedup_shape = "reviewing/<proposal_id>/<state.version>/<pr>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
      fact("review-converge-round", "marker-read"),
    },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      version = "marker:state.version",
      pr_number = "marker:pr-link.pr",
      source_ref = "source_ref:pr",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect({ "devloop_reviewing" }, "review payload is complete when current PR head is fetched and no head-bound review result exists"),
    marker_facts = "state:v1 reviewing plus PR head facts",
    kickoff = "devloop_reviewing",
    replay = "PR observe re-derives review kickoff from current PR head and issue version.",
  },
  {
    from_state = "fixing",
    to_states = { "reviewing", "review-meta" },
    driving_queue = "devloop_fixing",
    payload_builder = M.build_devloop_fixing_payload,
    dedup_shape = "forward:fixing/<proposal_id>/<version>/<pr>/<review_dedup>; replay:fixing/replay/<proposal_id>/<version>/<pr>/<review_dedup>/<gate_baseline_sha-or-nobase>/<reviewed_head_sha>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("review-result", "marker-read"),
      fact("review-meta", "marker-read"),
      fact("merge-gate", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
    },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      pr_number = "marker:pr-link.pr",
      version = "marker:state.version",
      review_proposal_id = "marker:merge-gate.review_proposal",
      review_dedup_key = "marker:merge-gate.review_dedup",
      reviewed_head_sha = "marker:merge-gate.head_sha",
      gate_baseline_sha = "marker:merge-gate.gate_baseline_sha",
      blocking_gap = "marker:review-result.gap",
      source_ref = "source_ref:pr",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect({ "devloop_fixing" }, "fixing replay is complete only when trusted feedback marker fields are copied into devloop_fixing"),
    marker_facts = "state:v1 fixing plus review-result/review-meta/merge-gate feedback, or current PR head for deterministic renormalization",
    kickoff = "devloop_fixing or devloop_reviewing",
    replay = "Observe re-raises fix when a trusted feedback fact is parseable; otherwise it re-enters reviewing for the current head.",
  },
  {
    from_state = "review-meta",
    to_states = { "fixing", "blocked" },
    driving_queue = "devloop_review_meta",
    payload_builder = M.build_devloop_review_meta_payload,
    dedup_shape = "review-meta/<proposal_id>/<version>/<pr>/<n>/<review_dedup>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("review-meta", "marker-read"),
      fact("fix-reflection", "marker-read"),
      fact("review-result", "marker-read"),
      fact("review-converge-round", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
    },
    payload_fields = {
      proposal_id = "marker:review-meta.proposal",
      review_proposal_id = "marker:review-converge-round.proposal",
      review_dedup_key = "marker:review-converge-round.dedup",
      version = "marker:state.version",
      pr_number = "marker:pr-link.pr",
      n = "marker:review-converge-round.round",
      blocking_gap = "marker:review-result.gap",
      source_ref = "source_ref:pr",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect({ "devloop_review_meta" }, "review-meta replay is complete when review proposal, dedup, PR number, and issue version are reconstructed"),
    marker_facts = "state:v1 review-meta plus review proposal encoded in version/dedup",
    kickoff = "devloop_review_meta",
    replay = "Observe re-raises review-meta using the review proposal, PR number, issue version, and original dedup.",
  },
  {
    from_state = "merge-ready",
    to_states = { "reviewing", "merging", "fixing", "blocked" },
    driving_queue = "devloop_merge_ready",
    payload_builder = M.build_devloop_merge_ready_payload,
    dedup_shape = "merge-ready/<proposal_id>/<version>/<pr>/<review_dedup>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("review-result", "marker-read"),
      fact("merge-ready", "marker-read"),
      fact("review-carry-over", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
      fact("base-head", "fetch-before-compare"),
    },
    payload_fields = {
      proposal_id = "marker:merge-ready.proposal",
      pr_number = "marker:merge-ready.pr",
      version = "marker:merge-ready.version",
      review_proposal_id = "marker:merge-ready.review_proposal",
      review_dedup_key = "marker:merge-ready.review_dedup",
      reviewed_head_sha = "marker:merge-ready.head_sha",
      source_ref = "source_ref:pr",
    },
    version_identity = "strip_transition_version_suffixes(merge-ready.version)",
    effects = effect({ "devloop_merge_ready" }, "merge-ready replay is complete when head-bound approval and fetched PR head match"),
    marker_facts = "state:v1 merge-ready plus merge-ready:v1",
    kickoff = "devloop_merge_ready",
    replay = "PR observe or merge retry re-derives merge-ready from head-bound approval facts.",
  },
  {
    from_state = "merging",
    to_states = { "merged", "fixing", "blocked" },
    driving_queue = "devloop_merge_ready",
    payload_builder = M.build_devloop_merge_ready_payload,
    dedup_shape = "merge-ready/<proposal_id>/<version>/<pr>/<review_dedup>",
    required_facts = {
      fact("state", "marker-read"),
      fact("merge-ready", "marker-read"),
      fact("merging", "marker-read"),
      fact("review-result", "marker-read"),
      fact("pr-head", "fetch-before-compare"),
      fact("ci-status", "fetch-before-compare"),
    },
    payload_fields = {
      proposal_id = "marker:merge-ready.proposal",
      pr_number = "marker:merge-ready.pr",
      version = "marker:merge-ready.version",
      review_proposal_id = "marker:merge-ready.review_proposal",
      review_dedup_key = "marker:merge-ready.review_dedup",
      reviewed_head_sha = "marker:merge-ready.head_sha",
      source_ref = "source_ref:pr",
    },
    version_identity = "strip_transition_version_suffixes(merge-ready.version)",
    effects = effect({ "devloop_merge_ready" }, "merging retry is complete when merge-ready and merging markers bind the same fetched PR head"),
    marker_facts = "state:v1 merging plus merging:v1",
    kickoff = "devloop_merge_ready",
    replay = "Merge retry re-derives completion or repair from PR mergeability and head facts.",
  },
  {
    from_state = "blocked",
    to_states = {},
    driving_queue = "devloop_decompose",
    payload_builder = M.build_decompose_replay_payload,
    dedup_shape = "forward:decompose/<proposal_id>/<version>; replay:decompose/replay/<proposal_id>/<version>/<pr>/<expected_child_count>/<completed_child_count>",
    required_facts = {
      fact("state", "marker-read"),
      fact("pr-link", "marker-read"),
      fact("decomposed", "marker-read"),
      fact("decompose-children", "fetch-before-compare"),
    },
    payload_fields = {
      proposal_id = "marker:state.proposal",
      version = "marker:state.version",
      pr_number = "marker:pr-link.pr",
      source_ref = "source_ref:pr",
    },
    version_identity = "strip_transition_version_suffixes(state.version)",
    effects = effect(
      { "decomposed-marker", "github-proxy.github_issue_create_request[*]" },
      "blocked decompose replay is complete only when the decomposed marker count and every declared child issue are derivable",
      "decompose_children_complete"
    ),
    marker_facts = "state:v1 blocked plus decomposed:v1 when class decomposition is incomplete",
    kickoff = "devloop_decompose",
    replay = "Observe can replay decomposed blocked issues when deterministic child completion facts are missing.",
  },
}

local audit_by_state = {}
for _, row in ipairs(transition_table) do
  audit_by_state[row.from_state] = row
end

function M.restart_completeness_audit()
  local rows = {}
  for _, row in ipairs(transition_table) do
    table.insert(rows, {
      state = row.from_state,
      marker_facts = row.marker_facts,
      kickoff = row.kickoff,
      replay = row.replay,
    })
  end
  return rows
end

function M.restart_completeness_audit_for_state(state)
  return audit_by_state[state]
end

function M.restart_transition_table()
  return transition_table
end

function M.restart_durable_marker_fields()
  return marker_fields
end

function M.restart_source_ref_derivations()
  return source_ref_derivations
end

function M.restart_required_replay_payload_fields()
  return required_replay_payload_fields
end

local function field_reference_error(reference)
  local marker_family, attr = tostring(reference or ""):match("^marker:([^%.]+)%.(.+)$")
  if marker_family ~= nil then
    if marker_fields[marker_family] == nil then
      return "unknown marker family " .. marker_family
    end
    if marker_fields[marker_family][attr] ~= true then
      return "unknown marker attr " .. marker_family .. "." .. attr
    end
    return nil
  end
  local derivation = tostring(reference or ""):match("^source_ref:(.+)$")
  if derivation ~= nil then
    if source_ref_derivations[derivation] == true then
      return nil
    end
    return "unknown source_ref derivation " .. derivation
  end
  return "unsupported payload field source " .. tostring(reference)
end

function M.restart_field_coverage_errors(rows)
  local errors = {}
  for _, row in ipairs(rows or transition_table) do
    local required_fields = required_replay_payload_fields[row.from_state] or {}
    for field, reason in pairs(required_fields) do
      if (row.payload_fields or {})[field] == nil then
        table.insert(errors, tostring(row.from_state or "?") .. "." .. tostring(field) .. ": missing required replay payload field: " .. tostring(reason))
      end
    end
    for field, reference in pairs(row.payload_fields or {}) do
      local err = field_reference_error(reference)
      if err ~= nil then
        table.insert(errors, tostring(row.from_state or "?") .. "." .. tostring(field) .. ": " .. err)
      end
    end
  end
  return errors
end

local default_consumer_sources = {
  "packages/github-devloop/departments/consensus_result/main.lua",
  "packages/github-devloop/departments/decompose/main.lua",
  "packages/github-devloop/departments/observe_pr/main.lua",
  "packages/github-devloop/departments/observe_issue/main.lua",
}

local function source_contains_any(paths, needle)
  if needle == nil or needle == "" then
    return false
  end
  for _, path in ipairs(paths or {}) do
    local ok, text = pcall(file.read, path)
    if ok and tostring(text or ""):find(tostring(needle), 1, true) ~= nil then
      return true
    end
  end
  return false
end

function M.restart_effect_contract_errors(rows, consumer_sources)
  local errors = {}
  local sources = consumer_sources or default_consumer_sources
  for _, row in ipairs(rows or transition_table) do
    local effects = row.effects or {}
    local kinds = effects.kinds or {}
    local count = tonumber(effects.intent_count) or #kinds
    if count > 1 then
      if type(kinds) ~= "table" or #kinds ~= count then
        table.insert(errors, tostring(row.from_state or "?") .. ": multi-effect row must enumerate declared effects")
      end
      if type(effects.completeness_derivation) ~= "string" or effects.completeness_derivation == "" then
        table.insert(errors, tostring(row.from_state or "?") .. ": multi-effect row must declare a completeness derivation")
      elseif not source_contains_any(sources, effects.completeness_derivation) then
        table.insert(errors, tostring(row.from_state or "?") .. ": completeness derivation is not called by consumer sources")
      end
    end
  end
  return errors
end

function M.latest_complete_converge_round(comments, proposal_id, base_version, source_ref)
  local sr_digest = M.source_ref_digest(source_ref)
  local latest = nil
  local facts = base_version ~= nil
    and M.converge_round_facts(comments, proposal_id, base_version, sr_digest)
    or M.converge_round_facts_for_source(comments, proposal_id, sr_digest)
  for _, fact in ipairs(facts) do
    if fact.narrowed_question ~= nil
      and fact.narrowed_question ~= ""
      and type(fact.angle_digests) == "table"
      and #fact.angle_digests > 0
      and (latest == nil or fact.round > latest.round) then
      latest = fact
    end
  end
  return latest
end

local function review_meta_fact_from_converge_marker(M, comments, issue_proposal_id, issue_version)
  if type(comments) ~= "table" then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-converge%-round:v1.-%-%->"
  local best = nil
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('issue_proposal="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      local review_proposal = marker:match('proposal="([^"]+)"')
      local consensus_dedup = marker:match('dedup="([^"]*)"')
      local round = tonumber(marker:match('round="(%d+)"'))
      local _, pr_number, review_version = M.parse_pr_review_proposal_id(review_proposal)
      local repo = M.parse_proposal_id(issue_proposal_id)
      if marker_issue == tostring(issue_proposal_id)
        and marker_version == tostring(issue_version)
        and review_version == M.safe_version_segment(issue_version)
        and repo ~= nil
        and M._is_positive_pr_number(pr_number)
        and M._is_path_safe_key(review_proposal, M._max_key_len)
        and M._is_bounded_string(consensus_dedup, M._max_dedup_len)
        and (best == nil or (round or 0) > (best.n or 0)) then
        best = {
          proposal_id = review_proposal,
          dedup_key = consensus_dedup,
          source_ref = M.pr_source_ref(repo, pr_number),
          pr_number = tonumber(pr_number),
          n = (round or 0) + 1,
        }
      end
    end
  end
  return best
end

function M.review_meta_replay_fact_from_state(comments, issue_proposal_id, issue_version, pr_number, head_sha, n)
  local repo = M.parse_proposal_id(issue_proposal_id)
  if repo == nil
    or not M._is_positive_pr_number(pr_number)
    or not M._is_git_sha(head_sha)
    or not M._is_bounded_string(issue_version, M._max_dedup_len) then
    return nil
  end
  local marker_pattern = "<!%-%- fkst:github%-devloop:review%-meta:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      local review_proposal = marker_dedup ~= nil and marker_dedup:match("^consensus:([^/].-)/review") or nil
      local _, review_pr_number, review_version, reviewed_head_sha = M.parse_pr_review_proposal_id(review_proposal)
      if marker_issue == tostring(issue_proposal_id)
        and tostring(review_pr_number or "") == tostring(pr_number)
        and review_version == M.safe_version_segment(M._strip_latest_fix_version_suffix(issue_version))
        and tostring(reviewed_head_sha or "") == tostring(head_sha)
        and M.is_safe_pr_review_result_ref(review_proposal, marker_dedup) then
        return {
          proposal_id = review_proposal,
          dedup_key = marker_dedup,
          source_ref = M.pr_source_ref(repo, pr_number),
          pr_number = tonumber(pr_number),
          n = tonumber(n) or 0,
        }
      end
    end
  end
  marker_pattern = "<!%-%- fkst:github%-devloop:fix%-reflection:v1.-%-%->"
  for _, comment in ipairs(M._trusted_marker_comments(comments)) do
    for marker in M._comment_body(comment):gmatch(marker_pattern) do
      local marker_issue = marker:match('proposal="([^"]+)"')
      local marker_dedup = marker:match('dedup="([^"]*)"')
      local verdict = marker:match('verdict="([^"]+)"')
      local marker_version = marker:match('version="([^"]*)"')
      local round = tonumber(marker:match('fix_round="(%d+)"'))
      local review_proposal = marker_dedup ~= nil and marker_dedup:match("^consensus:([^/].-)/review") or nil
      local _, review_pr_number, review_version, reviewed_head_sha = M.parse_pr_review_proposal_id(review_proposal)
      if marker_issue == tostring(issue_proposal_id)
        and verdict == "checkpoint"
        and marker_version == tostring(issue_version)
        and tostring(review_pr_number or "") == tostring(pr_number)
        and review_version == M.safe_version_segment(M._strip_latest_fix_version_suffix(issue_version))
        and tostring(reviewed_head_sha or "") == tostring(head_sha)
        and M.is_safe_pr_review_result_ref(review_proposal, marker_dedup) then
        local reject_fact = M.review_reject_fact(comments, issue_proposal_id, issue_version)
        if reject_fact == nil
          or tostring(reject_fact.review_proposal_id or "") ~= tostring(review_proposal)
          or tostring(reject_fact.review_dedup_key or "") ~= tostring(marker_dedup)
          or not M._is_bounded_string(reject_fact.blocking_gap, M._max_blocking_gap_len) then
          return nil
        end
        local reflection_dedup = M.fix_reflection_dedup_key(issue_proposal_id, issue_version, pr_number, round, marker_dedup)
        return {
          proposal_id = review_proposal,
          dedup_key = reflection_dedup,
          review_dedup_key = marker_dedup,
          source_ref = M.pr_source_ref(repo, pr_number),
          pr_number = tonumber(pr_number),
          n = tonumber(n) or 0,
          mode = "fix-reflection",
          fix_round = round,
          blocking_gap = reject_fact.blocking_gap,
        }
      end
    end
  end
  local reject_fact = M.review_reject_fact(comments, issue_proposal_id, issue_version)
  local _, reject_pr_number, _, reviewed_head_sha = M.parse_pr_review_proposal_id(reject_fact and reject_fact.review_proposal_id)
  if reject_fact ~= nil
    and tostring(reject_pr_number or "") == tostring(pr_number)
    and tostring(reviewed_head_sha or "") == tostring(head_sha)
    and M.is_safe_pr_review_result_ref(reject_fact.review_proposal_id, reject_fact.review_dedup_key) then
    return {
      proposal_id = reject_fact.review_proposal_id,
      dedup_key = reject_fact.review_dedup_key,
      source_ref = M.pr_source_ref(repo, pr_number),
      pr_number = tonumber(pr_number),
      n = tonumber(n) or 0,
    }
  end
  return nil
end

function M.review_meta_replay_fact(comments, issue_proposal_id, issue_version, pr_number, head_sha)
  local converge_fact = review_meta_fact_from_converge_marker(M, comments, issue_proposal_id, issue_version)
  if converge_fact ~= nil then
    return converge_fact
  end
  return M.review_meta_replay_fact_from_state(comments, issue_proposal_id, issue_version, pr_number, head_sha, 0)
end

function M.fixing_replay_feedback_fact(comments, issue_proposal_id, issue_version)
  local reject_fact = M.review_reject_fact(comments, issue_proposal_id, issue_version)
  if reject_fact ~= nil then
    return reject_fact
  end
  local meta_fix_fact = M.review_meta_fix_fact(comments, issue_proposal_id, issue_version)
  if meta_fix_fact ~= nil then
    return meta_fix_fact
  end
  return M.merge_gate_fix_fact(comments, issue_proposal_id, issue_version)
end

function M.fixing_version_matches_link(issue_version, link_version)
  local current = tostring(issue_version or "")
  local linked = tostring(link_version or "")
  if current == linked or M._strip_latest_fix_version_suffix(current) == linked then
    return true
  end
  local current_base = M.strip_transition_version_suffixes(current)
  local linked_base = M.strip_transition_version_suffixes(linked)
  if current_base == "" or linked_base == "" then
    return false
  end
  return M.safe_version_segment(current_base) == M.safe_version_segment(linked_base)
end

end

return S
