return {
  template = [[Judge this proposal from one consensus angle.
{{bias}}

Respond with exactly two lines and no other text.
Line one: the marker ⟦FKST:VERDICT⟧ followed by one word - approve, reject, or abstain.
Line two: the marker ⟦FKST:REPLY⟧ followed by one concise paragraph.

Proposal:
Angle: {{angle}}
Title: {{title}}
Body:
{{body}}
{{context_block}}]],

  bias = {
    minimal = "Bias: minimal. Prefer the smallest viable decision, reject unnecessary scope, and approve only when the proposal is clear and low-risk.",
    structural = "Bias: structural. Judge whether the proposal preserves clean boundaries, reliable data flow, and maintainable contracts.",
    delete = "Bias: delete. Prefer removing scope, indirection, or brittle behavior unless the proposal proves the added surface is necessary.",
  },
}
