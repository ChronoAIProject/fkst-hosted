local source_refs = require("contract.source_ref")

return function(M)
function M.is_supported_pr(payload)
  return type(payload) == "table"
    and payload.schema == "github-proxy.v1"
    and payload.type == "pr"
    and payload.repo ~= nil
    and M.is_safe_pr_number(payload.number)
    and source_refs.has_bounded_source_ref(payload.source_ref, M._max_key_len)
end
end
