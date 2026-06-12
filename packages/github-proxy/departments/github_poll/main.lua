local core = require("core")

local M = {}

M.spec = {
  consumes = { "github_poll_tick" },
  produces = { "github_entity_changed" },
  stall_window = "30s",
}

local entity_types = {
  { type = "issue", cmd = core.gh_issue_list_cmd },
  { type = "pr", cmd = core.gh_pr_list_cmd },
}

local function replay_sort_key(entity)
  return tostring(entity.updated_at or "") .. "/" .. string.format("%010d", tonumber(entity.number) or 0)
end

local function collect_changed(repo, entity_type, entities, fresh_changes, replay_candidates)
  for _, entity in ipairs(entities) do
    local key = core.entity_cache_key(repo, entity_type, entity.number)
    local cached_updated_at = cache_get(key)
    if cached_updated_at ~= entity.updated_at then
      local item = {
        entity_type = entity_type,
        entity = entity,
        key = key,
        replay = cached_updated_at == nil,
      }
      if item.replay then
        table.insert(replay_candidates, item)
      else
        table.insert(fresh_changes, item)
      end
    end
  end
end

local function replay_allowance(replay_candidates, budget)
  table.sort(replay_candidates, function(left, right)
    return replay_sort_key(left.entity) < replay_sort_key(right.entity)
  end)
  local allowed = {}
  for index = 1, math.min(#replay_candidates, budget) do
    table.insert(allowed, replay_candidates[index])
  end
  return allowed
end

local function raise_changed_item(repo, item)
  with_lock(item.key, function()
    local entity = item.entity
    if cache_get(item.key) ~= entity.updated_at then
      local dedup_key = core.entity_dedup_key(repo, item.entity_type, entity.number, entity.updated_at)
      -- At-least-once: raise before cache_set. If this process crashes
      -- before the write, the next tick raises the same dedup_key again.
      raise("github_entity_changed", {
        schema = "github-proxy.v1",
        type = item.entity_type,
        repo = repo,
        number = entity.number,
        title = entity.title,
        url = entity.url,
        state = entity.state,
        labels = entity.labels,
        updated_at = entity.updated_at,
        view_cache_key = core.entity_view_cache_key(repo, item.entity_type, entity.number, entity.updated_at),
        dedup_key = dedup_key,
        source = "gh",
        -- Durable-delivery: stable pointer so a reliable consumer can
        -- re-derive the current entity (also required by the engine when
        -- this event is routed to a reliable subscription).
        source_ref = core.entity_source_ref(repo, item.entity_type, entity.number),
      })
      cache_set(item.key, entity.updated_at)
    end
  end)
end

local function raise_changed(repo, fresh_changes, replay_changes)
  for _, item in ipairs(fresh_changes or {}) do
    raise_changed_item(repo, item)
  end
  for _, item in ipairs(replay_changes or {}) do
    raise_changed_item(repo, item)
  end
end

local function poll_entities(repo, event, fresh_changes, replay_candidates)
  for _, entity_type in ipairs(entity_types) do
    local ok, result_or_err = core.gh_exec_result(entity_type.cmd(repo), 30, "gh " .. entity_type.type .. " list")
    if not ok then
      core.log_error_fact("warn", "github_poll", "FAILURE", result_or_err.class, event and event.queue, result_or_err.message, {
        source_ref = event and event.source_ref,
        attempt = event and event.attempt,
        terminal = false,
      })
      if core.is_gh_rate_limit_error(result_or_err) then
        error(result_or_err.message)
      end
    else
      collect_changed(repo, entity_type.type, core.parse_entity_list(result_or_err.stdout, entity_type.type), fresh_changes, replay_candidates)
    end
  end
end

function pipeline(event)
  local repo = core.read_env("FKST_GITHUB_REPO")
  if repo == nil then
    log.warn("github-proxy: FKST_GITHUB_REPO missing; skipping poll")
    return
  end

  local replay_budget = core.devloop_replay_budget()
  local fresh_changes = {}
  local replay_candidates = {}
  poll_entities(repo, event, fresh_changes, replay_candidates)
  raise_changed(repo, fresh_changes, replay_allowance(replay_candidates, replay_budget))
end

pipeline = core.wrap_pipeline_failure("github_poll", pipeline)

return M
