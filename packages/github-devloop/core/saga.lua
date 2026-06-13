local S = {}

function S.install(M)
-- Saga versions are CAS order tokens: they move forward as the durable state
-- stream advances and let independent workers compare which fact wins.
-- Effect ids are stable content fingerprints: they name the external effect
-- we may need to perform more than once until its durable completion fact is
-- visible. An idempotent effect must therefore be guarded by completion_check,
-- not by a write-once "started" marker.

local function validate_effect_once_opts(opts)
  if type(opts) ~= "table" then
    error("github-devloop: saga.effect_once requires opts")
  end
  if not M._is_bounded_string(opts.effect_id, M._max_dedup_len) then
    error("github-devloop: saga.effect_once requires a stable effect_id")
  end
  if type(opts.completion_check) ~= "function" then
    error("github-devloop: saga.effect_once requires completion_check")
  end
  if type(opts.perform) ~= "function" then
    error("github-devloop: saga.effect_once requires perform")
  end
end

function M.effect_once(opts)
  validate_effect_once_opts(opts)
  if opts.completion_check() then
    return {
      effect_id = opts.effect_id,
      action = "skip",
    }
  end
  return {
    effect_id = opts.effect_id,
    action = "perform",
    result = opts.perform(),
  }
end

end

return S
