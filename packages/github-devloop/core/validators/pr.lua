return function(M)
function M.is_supported_pr(payload)
  return type(payload) == "table"
    and payload.schema == "github-proxy.v1"
    and payload.type == "pr"
    and payload.repo ~= nil
    and M.is_safe_pr_number(payload.number)
    and M._has_bounded_source_ref(payload.source_ref)
end
end
