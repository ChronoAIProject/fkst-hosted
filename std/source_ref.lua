-- std.source_ref: structural helpers for stable {kind, ref} source pointers.
local strings = require("std.strings")

local R = {}

function R.has_bounded_source_ref(source_ref, limit)
  return type(source_ref) == "table"
    and strings.is_bounded_string(source_ref.kind, limit)
    and strings.is_bounded_string(source_ref.ref, limit)
end

return R
