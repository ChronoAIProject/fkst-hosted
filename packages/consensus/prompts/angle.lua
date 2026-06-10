return {
  template = [[Judge this proposal from one consensus angle.
{{bias}}

Execution boundary:
- You are running in an empty runtime scratch directory, not a repository checkout.
- Do not clone, checkout, fetch with git, create branches, or modify any repository.
- Fetch required source content only via the source_ref and fetch instruction below.

Respond with exactly the requested marker lines and no other text.
Line one: the marker ⟦FKST:VERDICT⟧ followed by one word - {{verdict_options}}.
Line two: the marker ⟦FKST:REPLY⟧ followed by one concise paragraph.
{{readiness_instruction}}

Proposal:
Angle: {{angle}}
Title: {{title}}
{{convergence_block}}
{{body_label}}
{{body}}
{{content_fetch_block}}
{{context_block}}]],

  bias = {
    minimal = "Bias: minimal. Prefer the smallest viable decision; treat unnecessary or unproven scope as a reason not to approve, and state in the reply what concrete evidence or scoping would change that.",
    structural = "Bias: structural. Judge whether the proposal preserves clean boundaries, reliable data flow, and maintainable contracts.",
    delete = "Bias: delete. Prefer removing scope, indirection, or brittle behavior unless the proposal proves the added surface is necessary.",
  },
}
