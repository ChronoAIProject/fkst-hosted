# Harness Philosophy: A Bounded Theory of Mechanical Conformance

## 1. Status, Scope, and Thesis

This document states a design stance and a hypothesis. It synthesizes established ideas; it neither proves a paradigm nor claims to invent one. Its prescriptions apply when an invariant is stable enough to name, consequential enough to protect, and bounded by identifiable authority, error, and threat surfaces. They become weaker as those conditions weaken.

**Thesis.** For a stable, consequential invariant, a harness places every relevant authoritative effect behind one governing decision procedure at the **smallest justified complete boundary under declared trade-offs**. The declared comparison criteria here are mediation coverage, trusted-base and authority-surface size, ownership alignment, and declared operational constraints and cost budget. Operational constraints may include latency, availability, throughput, and recovery objectives. These criteria may yield multiple incomparable, Pareto-efficient boundaries rather than one uniquely smallest boundary. The harness shifts the decidable part of conformance from repeated reasoner discipline to the strongest proportionate mechanical enforcement available and leaves the unmechanized residual explicit, evidenced, and adversarially governed. Complete mediation defines the harness as **no bypass of the governing procedure** within the declared boundary. It does not establish that the procedure, including any residual judgment it invokes, decides correctly. Correct satisfaction of the invariant is the separate, only partly mechanizable goal governed by A5. A mechanism that only detects known bypasses is a migration control or harness approximation, not a complete harness.

The core is mechanical substitution for repeated discipline. Artificial intelligence sharpens the motivation because an AI reasoner is visibly fallible, but it does not found the theory. Harness ideas predate AI and apply to any fallible reasoner, including individual humans, teams, and automated processes. This is not a theory of "programming for AI."

The claim is deliberately narrow. In this document, **harness** means a **mechanical conformance harness**. This stipulative use does not redefine every established use of the word, including ordinary test harnesses and conformance harnesses. A mechanical conformance harness governs a named invariant within a declared boundary. It does not establish that the invariant was chosen correctly, that the boundary is complete, or that the whole system is correct.

## 2. Definition and Axioms

A **mechanical conformance harness** is an arrangement of authority and enforcement for a named invariant. It identifies the effects that can uphold or violate that invariant, completely mediates those effects through one governing decision procedure within its declared boundary, mechanically rejects as much nonconformance as can be decided proportionately, and governs the unmechanized residual through evidence and accountable judgment. That residual comprises both intrinsically non-mechanizable judgment and mechanization proportionately deferred because its current cost exceeds the declared budget. Complete mediation supplies a no-bypass claim, not a guarantee that the governing procedure is correct.

The following axioms define this stance.

### A1. Bound the Claim

A harness governs a **named invariant** within a declared authority boundary and a declared error or threat boundary. It does not certify the whole system. Its assurance claim must state what is protected, which actors and effects are in scope, which assumptions are trusted, and which failures remain outside the claim. This follows the bounded obligations of **Design by Contract** and **formal specification**: a contract can support conclusions only about the properties and environment it actually specifies.

### A2. Minimal Canonical Authority and Complete Mediation

One canonical path owns the protected effect and receives only the authority it needs. Within the declared boundary, competing authority paths must be unreachable or unable to exercise the effect. Multiple compliant implementations may exist behind the path; canonicality governs the **authority path**, not a single implementation.

This axiom joins **capability security**, the **principle of least authority**, and **complete mediation** with the mistake-proofing aim of **poka-yoke** and the explicit obligations of **Design by Contract**. The intended shape is not merely "use the preferred interface." Every relevant attempt must pass through the governing procedure: no competing path may bypass its decision. That no-bypass property does not imply that the procedure makes the right decision. Its mechanically decidable checks and its fallible residual judgment must be evaluated separately under A3 and A5. Complete mediation is definitional here: a detection-only control that may miss an unknown bypass does not satisfy this axiom.

### A3. Strongest Proportionate Mediation

For comparable coverage and trusted base, use the strongest bypass-resistant mechanism proportionate to the invariant's consequence and stability. The tiers run from an incomplete migration approximation to increasingly strong forms of complete mediation:

1. a detection-only scan or ratchet that rejects a known bypass;
2. checked declarative or typed structure that completely mediates the protected effect and makes the illegal shape structurally inexpressible;
3. a runtime effect guard that mediates every relevant attempt and applies the governing safety contract;
4. capability restriction, where the bypass primitive is unreachable.

Tier 1 is a **migration control or harness approximation**, not a complete harness. It is useful until a mediating boundary at tier 2, 3, or 4 is available, but it must never be represented as complete mediation or prevention. This is an enforcement-strength heuristic, not a universal ordering of every type system, runtime guard, or verification method. Mechanisms may compose, and a mechanism with narrower coverage may be weaker in practice despite occupying a nominally stronger category.

The prevention lineage includes **type-driven design** and "make illegal states unrepresentable," mechanized invariants, and **poka-yoke**. **Property-based testing is a detection mechanism**: it searches for counterexamples but does not make untested invalid constructions impossible. **Continuous integration is an admission mechanism whose checks detect specified failures**; it blocks admission when one of those checks fires, but it is not prevention of the construction or existence of unknown nonconformance. These mechanisms can support a harness without being confused with complete mediation or prevention.

### A4. Explicit Authority and Traceable Disposition

Within the declared boundary, the authority responsible for a governed decision must be explicit. Failures and violations must leave a traceable disposition that identifies the accountable handler and outcome. This does not require a reconstruction trail for every conforming outcome, one file, or the absence of abstraction.

A2 requires complete mediation, but an opaque governing procedure can still obscure which authority exercised it or silently discard failures. A5 governs residual judgment, but it need not trace deterministic failures or violations. A4 therefore adds operational accountability without requiring general per-outcome auditability. It does not forbid state, encapsulation, polymorphism, abstraction, or plural sources resolved by an explicit authority rule.

### A5. Governed Residual

What remains unmechanized includes both judgment that is intrinsically non-mechanizable and mechanization proportionately deferred under the declared trade-offs. The system should preserve evidence rather than self-certify and seek genuinely independent adversarial and cross-perspective challenge. Where consequences cannot responsibly be delegated, it should retain an accountable human backstop. A low-consequence harness may proportionately terminate in an automated hold or failure. Where the governing contract is fail closed, missing required evidence means hold. These practices may raise confidence; none proves correctness.

Residual governance pairs two questions. **Beauty:** is the form faithful to the invariant's essence? **Worth:** is this amount and placement of enforcement proportionate to the risk and scope? Both questions require adversarial review rather than author self-grading.

This axiom is anchored in **independent verification and validation**. Its particular use of adversarial cross-perspective challenge, preserved evidence, and accountable human backstops is this document's synthesis. These practices support disciplined confidence claims, not certainty.

## 3. The Enforcement-Strength Gradient

The gradient compares candidate mechanisms only under explicit comparison assumptions: they address the same named invariant, cover materially the same authority surface, and rely on a materially comparable trusted base. Without those assumptions, the ordering can mislead.

**Scan or ratchet enforcement** detects known bypass forms and rejects their admission. It can be valuable as a migration control or regression backstop, particularly when a stronger boundary is not yet available. It remains detection: an unrecognized spelling, construction, or path may evade it. It is therefore a harness approximation, not a complete harness, and must not be described as prevention. An inventory shrinking toward zero is evidence of convergence, not proof that no unknown bypass exists.

**Checked declarative or typed structure** can prevent illegal forms covered by the checker. It is especially useful when conformance is a property of shape rather than environment. Its strength depends on what the structure expresses, what the checker proves, and whether authoritative effects remain reachable outside it. It constitutes a complete harness only when the protected effect cannot bypass the checked structure.

**Runtime effect guarding** is appropriate when the effect remains syntactically reachable but its admissibility depends on runtime facts. The guard supplies complete mediation only if it governs every relevant attempt and cannot itself be bypassed within the declared threat boundary. Its response follows the governing safety contract; fail-closed and fail-operational systems may require different responses while preserving the invariant.

**Capability restriction** is the target when the protected effect can be made available only through the canonical path. A bypass that lacks the primitive cannot exercise the authority. This can offer the strongest resistance to accidental or unauthorized alternatives within the capability boundary.

These mechanisms can be layered. A structural declaration may grant a capability; a runtime guard may validate dynamic facts; a scan may keep legacy bypasses from returning during migration. Composition is useful only when each layer has a distinct, stated job. Redundant layers without distinct coverage can enlarge the trusted base and create false confidence.

## 4. Why Structure Substitutes for Repeated Discipline

The core rationale is limited but practical: when conformance is decidable and the authority boundary is complete, a mechanical constraint can perform the same check on every governed exercise of authority. Repeated reasoner discipline instead requires each author, reviewer, and maintainer to remember, interpret, and apply the invariant again.

For stable, consequential invariants, that repetition creates opportunities for omission, drift, and plausible bypass. A harness can reduce those opportunities by making the compliant construction ordinary and the noncompliant construction unavailable or rejectable. It does not make the reasoner infallible. It changes where fallibility can act: from every exercise of authority toward the smaller tasks of choosing the invariant, locating the boundary, validating the mechanism, and governing the residual.

AI makes this trade-off easier to notice because generated work can be fluent while missing a local constraint. The same failure shape exists in human work: memory fades, teams turn over, conventions diverge, and review attention is finite. The hypothesis therefore rests on fallible reasoning in general. AI is an amplifier of the motivation, not its foundation.

## 5. Derived Consequences

These consequences follow from the axioms under their stated scope; they are not additional axioms.

### Unrepresentable Invalid States

When an invariant can be expressed as a construction rule, the preferred design makes invalid states or authority exercises unrepresentable within the governed interface. A post hoc rejection remains necessary when admissibility depends on dynamic facts, but it should not replace structural prevention for facts already known at construction time.

### One Explicit Authority Rule per Fact

For each fact that is authoritative within the declared scope, one explicit, traceable resolution rule should determine how authority is established. The rule may designate one source, a quorum, a consensus process, federated ownership, reconciliation, or another plural-authority arrangement. Representations such as projections, caches, evidence, and user interfaces must not silently replace or compete with that rule. A2 requires one canonical path for the protected effect; it does not require one source for every fact.

### Explicit Failure and Contract-Governed Recovery

An unhandled violation should remain visible until it reaches a handler that has both the authority and the policy to respond. The governing safety contract determines whether the handler must fail closed, continue operating under explicit degraded semantics, or take another defined action. Fail-closed behavior is required only where that contract requires it.

The governing liveness contract likewise determines recovery bounds. Where it requires bounded resolution, automated recovery must have a bound, and exhaustion should produce evidence and follow the defined escalation, hold, or failure path. Durable indefinite retry can be correct where the liveness contract permits it; a finite budget or escalation layer should not be invented merely to satisfy this theory.

## 6. The Unmechanized Residual

The unmechanized residual has two parts: **intrinsically non-mechanizable judgment**, including premises or domain truths that are undecidable, unobservable, or contested; and **proportionately deferred mechanization**, where enforcement is currently too expensive under the declared operational constraints and cost budget. The second part is contingent, not irreducible, and should be reconsidered when risks, constraints, or mechanization costs change.

No harness can mechanically choose all of its own premises. Someone must judge whether the invariant matters, whether the authority boundary is complete, whether the trusted base is acceptable, whether an enforcement mechanism covers what it claims, and whether its cost is justified.

The residual should therefore be governed by evidence rather than self-certification. Authors expose assumptions, scope, negative cases, and verification results. Independent reviewers try to falsify them. Cross-perspective review seeks different models, expertise, incentives, or failure hypotheses rather than multiple repetitions of the same reasoning. An accountable human remains the backstop for consequences that cannot responsibly be delegated.

Independence is a degree, not a label. Shared evidence, training, incentives, or framing can correlate reviewers. Adversarial and ensemble processes may raise confidence when they add genuinely different attack surfaces, but agreement alone is not proof. Under a fail-closed contract, absent required evidence means hold, not infer success from silence.

Beauty and Worth discipline the residual together:

- **Beauty asks:** does the form follow the invariant's real authority and purpose, or does it regulate a proxy, add arbitrary parameters, or catch symptoms after the fact?
- **Worth asks:** is the invariant stable and consequential enough for this mechanism, and is the mechanism placed at the **smallest justified complete boundary under declared trade-offs**? The declared comparison criteria here are full mediation coverage, trusted-base and authority-surface size, ownership alignment, and declared operational constraints and cost budget. Operational constraints may include latency, availability, throughput, and recovery objectives. Coverage is mandatory; the remaining criteria can trade off and can leave multiple incomparable, Pareto-efficient boundaries. The choice must not rely on speculative generalization or scope expansion.

Neither question is reliably answered by the author alone. Their value lies in making the grounds for adversarial challenge explicit.

## 7. Costs, Counter-Conditions, and Non-Guarantees

A harness has costs. It can add verbosity, rigidity, a larger trusted base, and an up-front design tax. It can slow legitimate evolution when the protected invariant changes. Most dangerously, it can create false confidence when its scope, coverage, or threat boundary is overstated. A canonical authority path can also be implemented as needless centralization when an explicit plural resolution rule would mediate the effect just as completely.

A lighter approach can be better for unstable exploratory work, disposable prototypes, one-off scripts, and low-consequence paths. In those settings, tests, review, simple assertions, or even an explicit convention may provide a better cost-to-risk ratio. If the invariant later stabilizes or its consequences grow, the proportional answer may change. The stance is prescriptive inside its applicability conditions, not a demand to mechanize every preference.

The non-guarantees are fundamental:

- A harness does not prove whole-system correctness.
- It does not prove that the named invariant is sufficient or desirable.
- It does not prove that the declared authority or threat boundary is complete.
- It does not eliminate defects in the enforcement mechanism or its trusted base.
- Evidence, independent review, ensembles, and human oversight can raise confidence but do not prove correctness.
- A detection gate remains detection even when admission depends on it; it is not prevention of unknown bypasses.

## 8. References and Provenance

The borrowed ideas and their use here are:

- **A1, bounded obligations:** Bertrand Meyer, "Applying 'Design by Contract'," *Computer* 25(10), 1992, and C. A. R. Hoare, "An Axiomatic Basis for Computer Programming," *Communications of the ACM* 12(10), 1969. They support explicit preconditions, postconditions, and bounded claims; they do not establish this document's harness thesis.
- **A2, no bypass and restricted authority:** Jerome H. Saltzer and Michael D. Schroeder, "The Protection of Information in Computer Systems," *Proceedings of the IEEE* 63(9), 1975, especially complete mediation and least privilege. Capability restriction also draws on Jack B. Dennis and Earl C. Van Horn, "Programming Semantics for Multiprogrammed Computations," *Communications of the ACM* 9(3), 1966. These sources motivate mediation and constrained authority, not guaranteed correctness of the mediator.
- **A2-A3, mistake-proofing:** Shigeo Shingo, *Zero Quality Control: Source Inspection and the Poka-Yoke System*, Productivity Press, 1986. Poka-yoke motivates preventing a known class of mistake by construction where feasible.
- **A3, type-driven prevention:** Yaron Minsky, "Effective ML" (talk and notes, 2011), articulates the type-driven maxim "make illegal states unrepresentable." This document applies that design instinct only within an explicitly bounded authority surface.
- **A3, detection and admission only:** Koen Claessen and John Hughes, "QuickCheck: A Lightweight Tool for Random Testing of Haskell Programs," *ICFP 2000*. Property-based testing detects counterexamples generated by its tests; it is **not prevention**. Martin Fowler, "Continuous Integration," 2006, describes an automated integration discipline; in this document a CI gate is an **admission mechanism backed by detection checks**, not proof and not prevention of unknown nonconformance.
- **A4, accountability; A5, independent challenge:** *IEEE Std 1012-2016, IEEE Standard for System, Software, and Hardware Verification and Validation*. Its independence, evidence, traceability, and V&V responsibilities inform these axioms. A4's explicit-authority and traceable-disposition requirement and A5's residual-governance formulation are this document's synthesis, not claims made verbatim by the standard.
- **Derived recovery consequence:** George Candea and Armando Fox, "Crash-Only Software," *HotOS IX*, 2003, motivates recovery-oriented components designed around crash and restart. Joe Armstrong, *Making Reliable Distributed Systems in the Presence of Software Errors*, PhD thesis, 2003, together with the Erlang/OTP supervisor principles, motivates supervision, failure propagation, and the "let it crash" stance. Neither source implies that every system should use bounded retry or fail closed; those choices remain contract-dependent here.
The **combined thesis, the axiom set, the enforcement gradient, and the particular separation of no-bypass mediation from fallible residual judgment are this document's hypothesis and synthesis**. No single source above proposes that combined theory. The synthesis is offered for evaluation: stable, consequential invariants may be upheld more reliably when decidable conformance is transferred from repeated discipline into proportionate structure, while remaining judgment is exposed and governed. Its value must be assessed against evidence in each domain. No claim of novelty, completeness, or proof is made.

---

⟦AI:FKST⟧
