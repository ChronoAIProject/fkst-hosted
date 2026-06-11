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

function pipeline(_event)
  local repo = core.read_env("FKST_GITHUB_REPO")
  if repo == nil then
    log.warn("github-proxy: FKST_GITHUB_REPO missing; skipping poll")
    return
  end

  for _, entity_type in ipairs(entity_types) do
    local ok, result_or_err = core.gh_exec_result(entity_type.cmd(repo), 30, "gh " .. entity_type.type .. " list")
    if not ok then
      if core.is_gh_rate_limit_error(result_or_err) then
        error(result_or_err.message)
      end
      log.warn(result_or_err.message)
    else
      local entities = core.parse_entity_list(result_or_err.stdout, entity_type.type)
      for _, entity in ipairs(entities) do
        local key = core.entity_cache_key(repo, entity_type.type, entity.number)
        with_lock(key, function()
          if cache_get(key) ~= entity.updated_at then
            local dedup_key = core.entity_dedup_key(repo, entity_type.type, entity.number, entity.updated_at)
            -- At-least-once: raise before cache_set. If this process crashes
            -- before the write, the next tick raises the same dedup_key again.
            raise("github_entity_changed", {
              schema = "github-proxy.v1",
              type = entity_type.type,
              repo = repo,
              number = entity.number,
              title = entity.title,
              url = entity.url,
              state = entity.state,
              labels = entity.labels,
              updated_at = entity.updated_at,
              dedup_key = dedup_key,
              source = "gh",
              -- Durable-delivery: stable pointer so a reliable consumer can
              -- re-derive the current entity (also required by the engine when
              -- this event is routed to a reliable subscription).
              source_ref = core.entity_source_ref(repo, entity_type.type, entity.number),
            })
            cache_set(key, entity.updated_at)
          end
        end)
      end
    end
  end
end

return M
