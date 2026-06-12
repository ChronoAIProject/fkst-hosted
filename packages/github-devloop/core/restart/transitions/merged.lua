return function(M, h)
  local fact = h.fact
  local obligation = h.obligation
  local effect = h.effect
  local budget = h.budget
  local timeout = h.timeout
  return {
    from_state = "merged",
    terminal = true,
    to_states = {},
    marker_facts = "state:v1 merged plus merged:v1",
    replay = "Merged is a legal terminal state and has no output obligation.",
  }
end
