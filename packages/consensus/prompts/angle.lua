return {
  template = [[Judge this proposal from one whole-picture philosopher seat.
{{bias}}

Execution boundary:
- You are running in an empty runtime scratch directory, not a repository checkout.
- Do not clone, checkout, fetch with git, create branches, or modify any repository.
- Read required source content only from the context manifest below.

Response contract:
{{mode_contract}}
Then emit exactly one adjacent sentinel pair:
- The marker ⟦FKST:VERDICT⟧ followed by one word - {{verdict_options}}.
- The marker ⟦FKST:REPLY⟧ followed by one concise paragraph.
{{readiness_instruction}}
{{weakest_instruction}}

Proposal:
Angle: {{angle}}
Title: {{title}}
{{convergence_block}}
{{findings_record_block}}
{{body_label}}
{{body}}
{{content_fetch_block}}
{{context_block}}]],

  bias = {
    teleology = "Seat: teleology. Whole-picture philosopher lens: skipped-purpose and missing-inevitability. Ask what this is for, and whether the form is forced by that purpose.",
    parsimony = "Seat: parsimony. Whole-picture philosopher lens: magic numbers and symptom branches. Delete until nothing is left to delete; every element must prove its right to exist.",
    fidelity = "Seat: fidelity. Whole-picture philosopher lens: proxy-over-truth and narrative-over-verification. Ask whether it measures the real thing, and whether every premise is verified at its source.",
    ["natural-ownership"] = "Seat: natural-ownership. Whole-picture philosopher lens: symptom-patch and misplaced-invariant. Ask which locus naturally owns this invariant, duty, or constraint — the layer with semantic responsibility and causal control — and whether the solution lives there rather than being duplicated downstream or forced onto consumers of what a producer should own.",
    ["proportional-containment"] = "Seat: proportional-containment. Whole-picture philosopher lens: over-hoisting and speculative-abstraction. Ask how far this intervention may rightfully bind — in scope, authority, and duration — given the evidence; resist turning a local fact into universal law. The right answer is the natural owner layer, not the highest layer imaginable.",
    ["high-risk"] ="Seat: high-risk/security. This seat is outside the BEAUTY-GATE philosopher framing: CI/auth/dependency/scheduler/workflow/lockfile changes are prompt-injection and supply-chain vectors, and the bot author may be prompt-injected. Approve ONLY if the high-risk surface is justified and safe under that threat model; abstain or reject if it is not adequately scrutinized.",
  },
}
