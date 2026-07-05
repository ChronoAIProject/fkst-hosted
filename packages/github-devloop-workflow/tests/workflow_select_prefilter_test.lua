local core = require("core")
local payloads_builders = require("devloop.payloads.builders")
local workflow_select = require("workflow_select")
local t = fkst.test

local function shell_quote(value)
  return "'" .. tostring(value):gsub("'", "'\\''") .. "'"
end

local function test_root()
  local token = tostring({}):gsub("[^A-Za-z0-9]", "")
  return "/tmp/fkst-workflow-select-prefilter-" .. token
end

local function cleanup(root)
  os.remove(root .. "/custom-flow.json")
  os.remove(root .. "/software-feature-flow.json")
  os.execute("rmdir " .. shell_quote(root) .. " >/dev/null 2>&1")
end

local function mkdir_p(path)
  local ok = os.execute("mkdir -p " .. shell_quote(path))
  if ok ~= true and ok ~= 0 then
    error("failed to create temp workflow catalog")
  end
end

local function with_catalog(files, fn)
  local root = test_root()
  cleanup(root)
  mkdir_p(root)
  for name, source in pairs(files or {}) do
    file.write(root .. "/" .. name, source)
  end
  local ok, err = pcall(function()
    fn(root)
  end)
  cleanup(root)
  if not ok then
    error(err, 0)
  end
end

local function workflow_json(id)
  return [[{
    "schema": "fkst.workflow.v1",
    "id": "]] .. id .. [[",
    "version": "1",
    "summary": "External workflow summary.",
    "applies_when": "The origin issue asks for this external workflow.",
    "selector": {"title_contains_any": ["external"]},
    "steps": [
      {"id":"first","title":"First external step","content":{"kind":"static","intent":"Implement the external workflow step."}}
    ]
  }]]
end

local function candidate()
  return payloads_builders.build_devloop_intake_candidate_payload("owner/repo", 42, "2026-06-03T01:02:03Z")
end

local function ctx_with_comments(comments)
  local payload = candidate()
  return {
    repo = "owner/repo",
    issue_number = 42,
    candidate = payload,
    current = {
      comments = comments or {},
    },
    workflow_catalog_root = "/tmp/fkst-workflow-prefilter-empty",
  }
end

local function with_raise_capture(fn)
  local previous = _G.raise
  local raised = {}
  _G.raise = function(queue, payload)
    table.insert(raised, {
      queue = queue,
      payload = payload,
    })
  end
  local ok, err = pcall(function()
    fn(raised)
  end)
  _G.raise = previous
  if not ok then
    error(err, 0)
  end
end

local function with_catalog_loader(loader, fn)
  local previous = workflow_select.load_catalog_for_ctx
  workflow_select.load_catalog_for_ctx = loader
  local ok, err = pcall(fn)
  workflow_select.load_catalog_for_ctx = previous
  if not ok then
    error(err, 0)
  end
end

local function blueprint(id, selector)
  return {
    id = id,
    summary = id .. " summary",
    applies_when = id .. " applies",
    selector = selector,
  }
end

return {
  test_existing_blueprint_is_the_only_handled_prefilter_path = function()
    local payload = candidate()
    local marker, err = core.marker.build_blueprint_marker(payload.proposal_id, "workflow-one", "digest-123")
    t.is_nil(err)
    t.is_true(core.marker.parse_blueprint_marker(marker, payload.proposal_id) ~= nil)

    t.eq(workflow_select.workflow_prefilter(ctx_with_comments({
      {
        body = marker,
        author_login = "fkst-test-bot",
      },
    })), true)

    t.eq(workflow_select.workflow_prefilter(ctx_with_comments({
      {
        body = marker,
        author_login = "someone-else",
      },
    })), false)

    t.eq(workflow_select.workflow_prefilter(ctx_with_comments({})), false)
  end,

  test_trusted_lineage_header_in_body_fast_paths_before_catalog = function()
    local payload = candidate()
    local lineage, err = core.marker.build_lineage_header("github-devloop/issue/owner/repo/7", "d-1234567890", "slot-one")
    t.is_nil(err)

    with_catalog_loader(function()
      error("catalog should not load for workflow descendants")
    end, function()
      with_raise_capture(function(raised)
        t.eq(workflow_select.workflow_prefilter({
          repo = "owner/repo",
          issue_number = 42,
          candidate = payload,
          current = {
            body = lineage .. "\n\nChild issue body.",
            comments = {},
            author_login = "fkst-test-bot",
          },
        }), true)
        t.eq(#raised, 2)
        t.eq(raised[1].queue, "github-proxy.github_issue_label_request")
        t.eq(raised[2].queue, "github-devloop.devloop_execute_request")
      end)
    end)
  end,

  test_lineage_header_in_trusted_comment_fast_paths_before_catalog = function()
    local payload = candidate()
    local lineage, err = core.marker.build_lineage_header("github-devloop/issue/owner/repo/7", "d-1234567890", "slot-one")
    t.is_nil(err)

    with_catalog_loader(function()
      error("catalog should not load for workflow descendants")
    end, function()
      with_raise_capture(function(raised)
        t.eq(workflow_select.workflow_prefilter({
          repo = "owner/repo",
          issue_number = 42,
          candidate = payload,
          current = {
            body = "ordinary body",
            author_login = "human",
            comments = {
              {
                body = lineage,
                author_login = "fkst-test-bot",
              },
            },
          },
        }), true)
        t.eq(#raised, 2)
        t.eq(raised[1].queue, "github-proxy.github_issue_label_request")
        t.eq(raised[2].queue, "github-devloop.devloop_execute_request")
      end)
    end)
  end,

  test_absent_or_untrusted_lineage_does_not_short_circuit_prefilter = function()
    local payload = candidate()
    local lineage, err = core.marker.build_lineage_header(payload.proposal_id, "d-1234567890", "slot-one")
    t.is_nil(err)
    local catalog_reads = 0

    with_catalog_loader(function()
      catalog_reads = catalog_reads + 1
      return {
        valid = {},
        errors = {},
        duplicates = {},
      }, "/tmp"
    end, function()
      t.eq(workflow_select.workflow_prefilter({
        candidate = payload,
        current = {
          comments = {},
        },
      }), false)
      t.eq(workflow_select.workflow_prefilter({
        candidate = payload,
        current = {
          body = lineage,
          author_login = "human",
          comments = {},
        },
      }), false)
      t.eq(workflow_select.workflow_prefilter({
        candidate = payload,
        current = {
          comments = {
            {
              body = lineage,
              author_login = "someone-else",
            },
          },
        },
      }), false)
    end)
    t.eq(catalog_reads, 3)
  end,

  test_origin_without_lineage_remains_selector_eligible = function()
    with_catalog_loader(function()
      return {
        valid = {
          matched = {
            path = "/tmp/workflow.json",
            blueprint = blueprint("matched", { labels_any = { "workflow" } }),
          },
        },
        errors = {},
        duplicates = {},
      }, "/tmp"
    end, function()
      local eligible = workflow_select.prefilter_eligible_blueprints({
        labels = { "workflow" },
      }, workflow_select.load_catalog_for_ctx({}))
      t.eq(#eligible, 1)
      t.eq(eligible[1].id, "matched")
    end)
  end,

  test_selector_prefilter_matches_labels_title_and_selectorless_blueprints = function()
    local catalog = {
      valid = {
        label = { blueprint = blueprint("label", { labels_any = { "workflow" } }) },
        title = { blueprint = blueprint("title", { title_contains_any = { "orchestrate" } }) },
        none = { blueprint = blueprint("none", nil) },
        empty = { blueprint = blueprint("empty", {}) },
        miss = { blueprint = blueprint("miss", { labels_any = { "other" }, title_contains_any = { "unrelated" } }) },
      },
    }

    local eligible = workflow_select.prefilter_eligible_blueprints({
      labels = { "workflow" },
      title = "Please orchestrate the release",
    }, catalog)
    local ids = {}
    for _, record in ipairs(eligible) do
      ids[#ids + 1] = record.id
    end

    t.eq(table.concat(ids, ","), "empty,label,none,title")
  end,

  test_bounded_workflow_select_offers_all_valid_blueprints_without_selector_filter = function()
    local catalog = {
      valid = {
        alpha = { blueprint = blueprint("alpha", { labels_any = { "alpha" } }) },
        beta = { blueprint = blueprint("beta", { title_contains_any = { "beta" } }) },
        gamma = { blueprint = blueprint("gamma", { labels_any = { "gamma" }, title_contains_any = { "gamma" } }) },
      },
    }

    local eligible = workflow_select.workflow_select_eligible_blueprints({
      labels = { "ordinary" },
      title = "文字太大了",
    }, catalog)
    local ids = {}
    for _, record in ipairs(eligible) do
      ids[#ids + 1] = record.id
    end

    t.eq(table.concat(ids, ","), "alpha,beta,gamma")
  end,

  test_large_workflow_select_catalog_keeps_selector_prefilter = function()
    local valid = {}
    for index = 1, workflow_select.MAX_WORKFLOW_SELECT_BLUEPRINTS + 1 do
      local id = string.format("flow-%03d", index)
      valid[id] = { blueprint = blueprint(id, { labels_any = { "miss" } }) }
    end
    valid["flow-007"].blueprint.selector = { labels_any = { "match" } }

    local eligible = workflow_select.workflow_select_eligible_blueprints({
      labels = { "match" },
      title = "No title keyword",
    }, { valid = valid })

    t.eq(#eligible, 1)
    t.eq(eligible[1].id, "flow-007")
  end,

  test_catalog_root_resolution_accepts_injected_temp_root = function()
    local root = "/tmp/fkst-workflow-catalog-root-injected"
    t.eq(workflow_select.resolve_catalog_root({ workflow_catalog_root = root .. "/" }), root)
  end,

  test_catalog_root_resolution_returns_nil_when_env_root_is_absent = function()
    local calls = {}
    local root = workflow_select.resolve_catalog_root({
      exec = function(command)
        calls[#calls + 1] = command
        if command == 'printf %s "$FKST_WORKFLOW_CATALOG_ROOT"' then
          return { stdout = "", stderr = "", exit_code = 0 }
        end
        return { stdout = "", stderr = "unexpected", exit_code = 1 }
      end,
    })

    t.is_nil(root)
    t.eq(calls[1], 'printf %s "$FKST_WORKFLOW_CATALOG_ROOT"')
    t.is_nil(calls[2])
  end,

  test_load_catalog_for_ctx_always_loads_builtin_default_without_external_root = function()
    local calls = {}
    local loaded, root = workflow_select.load_catalog_for_ctx({
      exec = function(command)
        calls[#calls + 1] = command
        return { stdout = "", stderr = "", exit_code = 0 }
      end,
    })

    t.is_nil(root)
    t.eq(calls[1], 'printf %s "$FKST_WORKFLOW_CATALOG_ROOT"')
    t.is_nil(calls[2])
    t.eq(#loaded.errors, 0)
    t.eq(loaded.valid["software-feature-flow"].path, "builtin:software-feature-flow")
    t.eq(loaded.valid["software-refactor-flow"].path, "builtin:software-refactor-flow")
    t.eq(loaded.valid["software-contract-migration-flow"].path, "builtin:software-contract-migration-flow")
    t.is_nil(loaded.valid["software-dev-flow"])
  end,

  test_load_catalog_for_ctx_merges_builtin_default_and_external_catalog = function()
    with_catalog({
      ["custom-flow.json"] = workflow_json("custom-flow"),
    }, function(root)
      local loaded, resolved = workflow_select.load_catalog_for_ctx({
        workflow_catalog_root = root,
      })

      t.eq(resolved, root)
      t.eq(#loaded.errors, 0)
      t.eq(loaded.valid["software-feature-flow"].path, "builtin:software-feature-flow")
      t.eq(loaded.valid["software-refactor-flow"].path, "builtin:software-refactor-flow")
      t.eq(loaded.valid["software-contract-migration-flow"].path, "builtin:software-contract-migration-flow")
      t.is_nil(loaded.valid["software-dev-flow"])
      t.is_true(loaded.valid["custom-flow"].path:sub(-16) == "custom-flow.json")
    end)
  end,

  test_load_catalog_for_ctx_duplicate_builtin_and_external_id_fails_closed = function()
    with_catalog({
      ["software-feature-flow.json"] = workflow_json("software-feature-flow"),
    }, function(root)
      local loaded = workflow_select.load_catalog_for_ctx({
        workflow_catalog_root = root,
      })

      t.is_nil(loaded.valid["software-feature-flow"])
      t.eq(#loaded.duplicates, 1)
      t.eq(loaded.duplicates[1].id, "software-feature-flow")
      t.eq(loaded.duplicates[1].paths[1], "builtin:software-feature-flow")
      t.is_true(loaded.duplicates[1].paths[2]:sub(-26) == "software-feature-flow.json")
      t.eq(loaded.errors[1].error.code, "duplicate_id")
    end)
  end,
}
