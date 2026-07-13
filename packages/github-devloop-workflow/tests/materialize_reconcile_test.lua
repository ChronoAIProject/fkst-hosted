local base_ids = require("devloop.base_ids")
local core = require("core")
local digest = require("core.digest")
local materialization = require("core.materialization")
local materialize_reconcile = require("materialize_reconcile")
local marker = require("core.marker")
local testing = require("testkit.testing")
local t = fkst.test

local repo = "owner/repo"
local origin_issue = 42
local origin = base_ids.proposal_id(repo, origin_issue)

local function blueprint()
  return {
    schema = "fkst.workflow.v1",
    id = "workflow-one",
    version = "2026-07-02",
    summary = "A bounded workflow.",
    applies_when = "The origin issue asks for this workflow.",
    steps = {
      {
        id = "first",
        title = "First static issue",
        content = {
          kind = "static",
          intent = "Implement the first static step.",
        },
      },
      {
        id = "second",
        title = "Second generated issue",
        content = {
          kind = "generated",
          generator = "Use the predecessor result to write the next issue.",
        },
      },
    },
  }
end

local function blueprint_marker()
  local built, err = marker.build_blueprint_marker(origin, "workflow-one", digest.blueprint_digest(blueprint()))
  t.is_nil(err)
  return built
end

local function comment(body)
  return {
    body = body,
    author_login = "fkst-test-bot",
    created_at = "2026-07-02T00:00:00Z",
  }
end

local function issue(comments, fields)
  local extra = fields or {}
  return {
    title = "Workflow origin",
    body = "Run the workflow.",
    state = extra.state or "OPEN",
    labels = extra.labels or {},
    assignees = { "fkst-test-bot" },
    author_login = "fkst-test-bot",
    comments = comments or { comment(blueprint_marker()) },
    repo = repo,
    number = origin_issue,
  }
end

local function event()
  return {
    queue = "github-devloop-workflow.workflow_materialization_tick",
    payload = { schema = "github-devloop-workflow.materialization-tick.v1" },
    ts = "2026-07-02T00:00:00Z",
  }
end

local function generated_spec(slot, body)
  return {
    title = slot == "second" and "Generated child issue" or "First static issue",
    body = body or (slot == "second" and "Generated follow-up body." or "Implement the first static step."),
  }
end

local function build_entry(slot_id, predecessor_ref_digest, spec, child_issue, state)
  local slot = slot_id == "second" and blueprint().steps[2] or blueprint().steps[1]
  local entry = materialization.write_generated_entry(origin, digest.blueprint_digest(blueprint()), slot, predecessor_ref_digest, spec)
  local built, err = marker.build_materialization_marker(
    origin,
    entry.blueprint_digest,
    entry.slot,
    entry.predecessor_ref_digest,
    entry.gen_contract_digest,
    entry.gen_spec_digest,
    entry.child_dedup,
    child_issue ~= nil and tostring(child_issue) or nil,
    state or "generated"
  )
  t.is_nil(err)
  return entry, built
end

local function generated_comment(slot_id, predecessor_ref_digest, spec)
  local _entry, built = build_entry(slot_id, predecessor_ref_digest, spec, nil, "generated")
  return comment(built)
end

local function created_comment(slot_id, predecessor_ref_digest, spec, child_issue)
  local _entry, built = build_entry(slot_id, predecessor_ref_digest, spec, child_issue, "created")
  return comment(built)
end

local function divergent_generated_comment(slot_id, predecessor_ref_digest, spec)
  local entry = materialization.write_generated_entry(
    origin,
    digest.blueprint_digest(blueprint()),
    slot_id == "second" and blueprint().steps[2] or blueprint().steps[1],
    predecessor_ref_digest,
    spec
  )
  local built, err = marker.build_materialization_marker(
    origin,
    entry.blueprint_digest,
    entry.slot,
    entry.predecessor_ref_digest,
    entry.gen_contract_digest,
    "d-9999999999",
    entry.child_dedup,
    nil,
    "generated"
  )
  t.is_nil(err)
  return comment(built)
end

local function parent_created_comment(entry, child_issue)
  return comment('<!-- fkst:github-proxy:issue-created:v1 dedup="' .. entry.child_dedup .. '" issue="' .. tostring(child_issue) .. '" -->')
end

local function parent_intent_comment(entry)
  return comment('<!-- fkst:github-proxy:issue-create-intent:v1 dedup="' .. entry.child_dedup .. '" -->')
end

local function child_body(slot_id, spec, child_dedup)
  local lineage = marker.build_lineage_header(origin, digest.blueprint_digest(blueprint()), slot_id)
  return lineage .. "\n\n" .. spec.body .. "\n\n<!-- fkst:github-proxy:issue-create:" .. child_dedup .. " -->"
end

local function child_body_with_blueprint(slot_id, spec, child_dedup, bp)
  local lineage = marker.build_lineage_header(origin, digest.blueprint_digest(bp or blueprint()), slot_id)
  return lineage .. "\n\n" .. spec.body .. "\n\n<!-- fkst:github-proxy:issue-create:" .. child_dedup .. " -->"
end

local function raise_capture(fn)
  local old_with_lock = with_lock
  with_lock = function(_key, locked)
    return locked()
  end
  local ok, result = pcall(fn)
  with_lock = old_with_lock
  if not ok then
    error(result, 0)
  end
  return result
end

local function run_with(fakes)
  local fake = fakes or {}
  local dept = require("workflow.saga").department({
    consumes = { "workflow_materialization_tick" },
    produces = {
      "github-proxy.github_issue_create_request",
      "github-proxy.github_issue_comment_request",
    },
    stall_window = "2m",
  }, materialize_reconcile.handlers(core, {
    deps = {
      read_repo = function()
        return repo
      end,
      list_open_issues = function()
        return fake.issues or { { number = origin_issue, title = "Workflow origin" } }
      end,
      read_issue = function()
        return fake.current or issue()
      end,
      verify_issue_claim = function()
        return fake.claim ~= false
      end,
      child_status = function(_core, child_ref)
        local key = tostring(child_ref.issue_number or child_ref.proposal_id or "")
        local result = (fake.child_statuses or {})[key] or fake.child_status or "running"
        if type(result) == "table" then
          return result.status, result.detail
        end
        return result
      end,
      load_blueprint = function(_ctx, workflow_id)
        if fake.workflow_missing then
          return nil
        end
        if tostring(workflow_id) ~= "workflow-one" then
          return nil
        end
        return {
          path = "test-workflow.json",
          blueprint = fake.blueprint or blueprint(),
        }
      end,
      spawn_codex = fake.spawn_codex,
      content_fetch = fake.content_fetch,
      release_done_claim = fake.release_done_claim or function()
        return true
      end,
      read_created_issue = fake.read_created_issue,
      search_created_issue = fake.search_created_issue or function()
        return nil
      end,
    },
  }))
  local result = raise_capture(function()
    return testing.run_fake(dept, event())
  end)
  return result.raises
end

local function only_queue(raised, queue)
  local out = {}
  for _, item in ipairs(raised or {}) do
    if item.queue == queue then
      out[#out + 1] = item
    end
  end
  return out
end

local tests = {
  test_static_frontier_raises_issue_create_directly_without_origin_spec = function()
    local raised = run_with()
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "github-proxy.github_issue_create_request")
    t.eq(raised[1].payload.parent, origin_issue)
    t.eq(raised[1].payload.parent_comment_target.repo, repo)
    t.eq(raised[1].payload.parent_comment_target.issue_number, origin_issue)
    t.is_true(raised[1].payload.body:find("fkst:github-devloop-workflow:lineage:v1", 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find("Implement the first static step.", 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find("fkst:github-devloop-workflow:materialization:v1", 1, true) == nil)
  end,

  -- Regression (found by real dogfood): a GENERATED first slot has no prior
  -- child; its predecessor result is the ORIGIN idea itself, so it must read the
  -- origin via content_fetch and generate — NOT error with missing-predecessor-result.
  test_generated_first_slot_reads_origin_not_missing_predecessor = function()
    local gen_bp = {
      schema = "fkst.workflow.v1",
      id = "workflow-one",
      version = "1",
      summary = "Generated first slot.",
      applies_when = "The origin idea.",
      steps = {
        { id = "first", title = "Analyze", content = { kind = "generated", generator = "Analyze the origin idea." } },
      },
    }
    local gen_marker = marker.build_blueprint_marker(origin, "workflow-one", digest.blueprint_digest(gen_bp))
    local fetched_ref = nil
    local raised = run_with({
      blueprint = gen_bp,
      current = issue({ comment(gen_marker) }),
      content_fetch = function(predecessor_ref, _ctx)
        fetched_ref = predecessor_ref and (predecessor_ref.source_ref or predecessor_ref) or nil
        return "runtime-cache:origin-content"
      end,
      spawn_codex = function()
        return { exit_code = 0, stdout = '{"title":"Architecture analysis","body":"Components and data flow."}' }
      end,
    })
    -- materializes (raises child create), not a terminal missing-predecessor error
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "github-proxy.github_issue_create_request")
    t.eq(raised[1].payload.parent_comment_target.repo, repo)
    t.eq(raised[1].payload.parent_comment_target.issue_number, origin_issue)
    t.eq(raised[1].payload.title, "Architecture analysis")
    t.is_true(raised[1].payload.body:find("Components and data flow.", 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find("missing-predecessor", 1, true) == nil)
    -- content_fetch was called with the ORIGIN's source_ref (predecessor is the origin, not nil)
    t.is_true(fetched_ref ~= nil)
    t.is_true(tostring(fetched_ref.ref or fetched_ref):find("#issue/" .. tostring(origin_issue), 1, true) ~= nil)
  end,

  test_generated_fact_raises_one_issue_create_request_with_lineage = function()
    local spec = generated_spec("first")
    local entry = build_entry("first", materialization.EMPTY_PREDECESSOR_REF_DIGEST, spec)
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        generated_comment("first", materialization.EMPTY_PREDECESSOR_REF_DIGEST, spec),
      }),
    })
    local creates = only_queue(raised, "github-proxy.github_issue_create_request")
    local comments = only_queue(raised, "github-proxy.github_issue_comment_request")
    t.eq(#raised, 1)
    t.eq(#creates, 1)
    t.eq(#comments, 0)
    t.eq(creates[1].payload.schema, "github-proxy.issue-create.v1")
    t.eq(creates[1].payload.dedup_key, entry.child_dedup)
    t.eq(creates[1].payload.parent, origin_issue)
    t.eq(creates[1].payload.parent_comment_target.repo, repo)
    t.eq(creates[1].payload.parent_comment_target.issue_number, origin_issue)
    t.is_true(creates[1].payload.body:find("fkst:github-devloop-workflow:lineage:v1", 1, true) ~= nil)
    t.is_true(creates[1].payload.body:find("Implement the first static step.", 1, true) ~= nil)
  end,

  test_parent_issue_created_marker_writes_created_materialization = function()
    local spec = generated_spec("first")
    local entry = build_entry("first", materialization.EMPTY_PREDECESSOR_REF_DIGEST, spec)
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        generated_comment("first", materialization.EMPTY_PREDECESSOR_REF_DIGEST, spec),
        parent_created_comment(entry, 108),
      }),
    })
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "github-proxy.github_issue_comment_request")
    t.is_nil(raised[1].payload.replace_marker)
    t.is_true(raised[1].payload.body:find("fkst:github-devloop-workflow:blueprint:v1", 1, true) == nil)
    t.is_true(raised[1].payload.body:find('state="created"', 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find('child_issue="108"', 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find(spec.title, 1, true) == nil)
    t.is_true(raised[1].payload.body:find(spec.body, 1, true) == nil)
  end,

  test_predecessor_merged_materializes_next_generated_slot = function()
    local first_spec = generated_spec("first")
    local first_pred = materialization.EMPTY_PREDECESSOR_REF_DIGEST
    local pred_ref = { kind = "external", ref = repo .. "#issue/108" }
    local predecessor_ref_digest = materialize_reconcile._private.predecessor_ref_digest({ source_ref = pred_ref })
    local seen_fetch = nil
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        created_comment("first", first_pred, first_spec, 108),
      }),
      child_statuses = { ["108"] = "result_ready" },
      content_fetch = function(ref)
        seen_fetch = ref.source_ref.ref
        return "runtime-cache:workflow/predecessor"
      end,
      spawn_codex = function(prompt)
        t.is_true(prompt:find("runtime-cache:workflow/predecessor", 1, true) ~= nil)
        return {
          exit_code = 0,
          stdout = '{"title":"Generated child issue","body":"Generated follow-up body."}',
        }
      end,
    })
    t.eq(seen_fetch, repo .. "#issue/108")
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "github-proxy.github_issue_create_request")
    t.eq(raised[1].payload.parent_comment_target.repo, repo)
    t.eq(raised[1].payload.parent_comment_target.issue_number, origin_issue)
    t.is_true(raised[1].payload.body:find("Generated follow-up body.", 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find('predecessor_ref_digest="' .. predecessor_ref_digest .. '"', 1, true) == nil)
  end,

  test_child_fatal_writes_blocked_terminal = function()
    local first_spec = generated_spec("first")
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        created_comment("first", materialization.EMPTY_PREDECESSOR_REF_DIGEST, first_spec, 108),
      }),
      child_statuses = { ["108"] = "fatal" },
    })
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "github-proxy.github_issue_comment_request")
    t.is_nil(raised[1].payload.replace_marker)
    t.is_true(raised[1].payload.body:find("terminal:v1", 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find('state="created"', 1, true) == nil)
    t.is_true(raised[1].payload.body:find('state="blocked"', 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find('reason_code="child-fatal-first"', 1, true) ~= nil)
  end,

  test_stale_child_fatal_terminal_rederives_merged_children_and_completes = function()
    local first_spec = generated_spec("first")
    local second_spec = generated_spec("second")
    local first_ref = { kind = "external", ref = repo .. "#issue/108" }
    local blocked_terminal, terminal_err = marker.build_terminal_marker(origin, "blocked", "child-fatal-second")
    t.is_nil(terminal_err)
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        created_comment("first", materialization.EMPTY_PREDECESSOR_REF_DIGEST, first_spec, 108),
        created_comment("second", materialize_reconcile._private.predecessor_ref_digest({ source_ref = first_ref }), second_spec, 109),
        comment(blocked_terminal),
      }),
      child_statuses = {
        ["108"] = "result_ready",
        ["109"] = "result_ready",
      },
    })

    t.eq(#raised, 1)
    t.eq(raised[1].queue, "github-proxy.github_issue_comment_request")
    t.is_true(raised[1].payload.body:find('state="done"', 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find('reason_code="all-slots-result-ready"', 1, true) ~= nil)
  end,

  test_non_first_no_changes_child_writes_blocked_terminal_with_why = function()
    local first_spec = generated_spec("first")
    local second_spec = generated_spec("second")
    local first_ref = { kind = "external", ref = repo .. "#issue/108" }
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        created_comment("first", materialization.EMPTY_PREDECESSOR_REF_DIGEST, first_spec, 108),
        created_comment("second", materialize_reconcile._private.predecessor_ref_digest({ source_ref = first_ref }), second_spec, 109),
      }),
      child_statuses = {
        ["108"] = "result_ready",
        ["109"] = {
          status = "fatal",
          detail = { impl_failed_reason = "no-changes" },
        },
      },
    })
    t.eq(#raised, 1)
    t.eq(raised[1].queue, "github-proxy.github_issue_comment_request")
    t.is_nil(raised[1].payload.replace_marker)
    t.is_true(raised[1].payload.body:find("terminal:v1", 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find('state="blocked"', 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find('reason_code="child-fatal-second-no-changes"', 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find('reason_code="all-slots-result-ready"', 1, true) == nil)
  end,

  test_all_slots_ready_writes_done_terminal = function()
    local first_spec = generated_spec("first")
    local second_spec = generated_spec("second")
    local first_ref = { kind = "external", ref = repo .. "#issue/108" }
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        created_comment("first", materialization.EMPTY_PREDECESSOR_REF_DIGEST, first_spec, 108),
        created_comment("second", materialize_reconcile._private.predecessor_ref_digest({ source_ref = first_ref }), second_spec, 109),
      }),
      child_statuses = {
        ["108"] = "result_ready",
        ["109"] = "result_ready",
      },
    })
    t.eq(#raised, 1)
    t.is_nil(raised[1].payload.replace_marker)
    t.is_true(raised[1].payload.body:find('state="done"', 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find('reason_code="all-slots-result-ready"', 1, true) ~= nil)
  end,

  test_wait_when_predecessor_running_raises_nothing = function()
    local first_spec = generated_spec("first")
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        created_comment("first", materialization.EMPTY_PREDECESSOR_REF_DIGEST, first_spec, 108),
      }),
      child_statuses = { ["108"] = "running" },
    })
    t.eq(#raised, 0)
  end,

  test_existing_child_search_records_created_without_second_create = function()
    local existing_spec = generated_spec("first")
    local generated_entry = materialization.write_generated_entry(
      origin,
      digest.blueprint_digest(blueprint()),
      blueprint().steps[1],
      materialization.EMPTY_PREDECESSOR_REF_DIGEST,
      existing_spec
    )
    local generator_calls = 0
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
      }),
      search_created_issue = function(_repo, child_dedup)
        t.eq(child_dedup, generated_entry.child_dedup)
        return {
          number = 108,
          title = existing_spec.title,
          body = child_body_with_blueprint("first", existing_spec, child_dedup, blueprint()),
          author_login = "fkst-test-bot",
        }
      end,
      spawn_codex = function()
        generator_calls = generator_calls + 1
        return {
          exit_code = 0,
          stdout = '{"title":"Divergent generated issue","body":"Divergent regenerated body."}',
        }
      end,
    })
    local creates = only_queue(raised, "github-proxy.github_issue_create_request")
    local comments = only_queue(raised, "github-proxy.github_issue_comment_request")
    t.eq(generator_calls, 0)
    t.eq(#creates, 0)
    t.eq(#comments, 1)
    t.is_nil(comments[1].payload.replace_marker)
    t.is_true(comments[1].payload.body:find('state="created"', 1, true) ~= nil)
    t.is_true(comments[1].payload.body:find('child_issue="108"', 1, true) ~= nil)
    t.is_true(comments[1].payload.body:find(existing_spec.title, 1, true) == nil)
    t.is_true(comments[1].payload.body:find(existing_spec.body, 1, true) == nil)
  end,

  test_parent_issue_created_marker_before_generator_records_created_without_codex_or_second_create = function()
    local existing_spec = generated_spec("first")
    local planned_entry = materialization.write_generated_entry(
      origin,
      digest.blueprint_digest(blueprint()),
      blueprint().steps[1],
      materialization.EMPTY_PREDECESSOR_REF_DIGEST,
      existing_spec
    )
    local generator_calls = 0
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        parent_created_comment(planned_entry, 108),
      }),
      read_created_issue = function(_repo, issue_number)
        t.eq(issue_number, "108")
        return {
          number = 108,
          title = existing_spec.title,
          body = child_body("first", existing_spec, planned_entry.child_dedup),
          author_login = "fkst-test-bot",
        }
      end,
      spawn_codex = function()
        generator_calls = generator_calls + 1
        return {
          exit_code = 0,
          stdout = '{"title":"Divergent generated issue","body":"Divergent regenerated body."}',
        }
      end,
    })
    local creates = only_queue(raised, "github-proxy.github_issue_create_request")
    local comments = only_queue(raised, "github-proxy.github_issue_comment_request")
    t.eq(generator_calls, 0)
    t.eq(#creates, 0)
    t.eq(#comments, 1)
    t.is_nil(comments[1].payload.replace_marker)
    t.is_true(comments[1].payload.body:find('state="created"', 1, true) ~= nil)
    t.is_true(comments[1].payload.body:find('child_issue="108"', 1, true) ~= nil)
    t.is_true(comments[1].payload.body:find(existing_spec.title, 1, true) == nil)
    t.is_true(comments[1].payload.body:find(existing_spec.body, 1, true) == nil)
  end,

  test_parent_issue_created_marker_unreadable_waits_without_codex_or_second_create = function()
    local existing_spec = generated_spec("first")
    local planned_entry = materialization.write_generated_entry(
      origin,
      digest.blueprint_digest(blueprint()),
      blueprint().steps[1],
      materialization.EMPTY_PREDECESSOR_REF_DIGEST,
      existing_spec
    )
    local generator_calls = 0
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        parent_created_comment(planned_entry, 108),
      }),
      read_created_issue = function()
        return nil
      end,
      spawn_codex = function()
        generator_calls = generator_calls + 1
        return {
          exit_code = 0,
          stdout = '{"title":"Divergent generated issue","body":"Divergent regenerated body."}',
        }
      end,
    })
    t.eq(generator_calls, 0)
    t.eq(#only_queue(raised, "github-proxy.github_issue_create_request"), 0)
    t.eq(#only_queue(raised, "github-proxy.github_issue_comment_request"), 0)
  end,

  test_parent_issue_create_intent_waits_without_codex_or_second_create = function()
    local existing_spec = generated_spec("first")
    local planned_entry = materialization.write_generated_entry(
      origin,
      digest.blueprint_digest(blueprint()),
      blueprint().steps[1],
      materialization.EMPTY_PREDECESSOR_REF_DIGEST,
      existing_spec
    )
    local generator_calls = 0
    local search_calls = 0
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        parent_intent_comment(planned_entry),
      }),
      search_created_issue = function(_repo, child_dedup)
        search_calls = search_calls + 1
        t.eq(child_dedup, planned_entry.child_dedup)
        return nil
      end,
      spawn_codex = function()
        generator_calls = generator_calls + 1
        return {
          exit_code = 0,
          stdout = '{"title":"Divergent generated issue","body":"Divergent regenerated body."}',
        }
      end,
    })
    t.eq(search_calls, 1)
    t.eq(generator_calls, 0)
    t.eq(#only_queue(raised, "github-proxy.github_issue_create_request"), 0)
    t.eq(#only_queue(raised, "github-proxy.github_issue_comment_request"), 0)
  end,

  test_second_run_with_created_ledger_is_noop = function()
    local stored_spec = generated_spec("first")
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        created_comment("first", materialization.EMPTY_PREDECESSOR_REF_DIGEST, stored_spec, 108),
      }),
    })
    t.eq(#only_queue(raised, "github-proxy.github_issue_create_request"), 0)
    t.eq(#only_queue(raised, "github-proxy.github_issue_comment_request"), 0)
  end,

  test_impossible_ledger_writes_error_terminal = function()
    local spec = generated_spec("second")
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        created_comment("second", materialization.EMPTY_PREDECESSOR_REF_DIGEST, spec, 109),
      }),
    })
    t.eq(#raised, 1)
    t.is_true(raised[1].payload.body:find('state="error"', 1, true) ~= nil)
    t.is_true(raised[1].payload.body:find('reason_code="impossible-ledger"', 1, true) ~= nil)
  end,

  test_done_terminal_releases_self_only_claim = function()
    local first_spec = generated_spec("first")
    local second_spec = generated_spec("second")
    local first_ref = { kind = "external", ref = repo .. "#issue/108" }
    local released = nil
    local raised = run_with({
      current = issue({
        comment(blueprint_marker()),
        created_comment("first", materialization.EMPTY_PREDECESSOR_REF_DIGEST, first_spec, 108),
        created_comment("second", materialize_reconcile._private.predecessor_ref_digest({ source_ref = first_ref }), second_spec, 109),
      }),
      child_statuses = {
        ["108"] = "result_ready",
        ["109"] = "result_ready",
      },
      release_done_claim = function(_core, release_repo, release_issue, release_origin)
        released = {
          repo = release_repo,
          issue = release_issue,
          origin = release_origin,
        }
        return true
      end,
    })
    t.eq(#raised, 1)
    t.eq(released.repo, repo)
    t.eq(released.issue, origin_issue)
    t.eq(released.origin, origin)
  end,
}

return tests
