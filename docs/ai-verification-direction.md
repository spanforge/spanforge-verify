# Product direction: AI testing and verification

Approved direction: 6 October 2026. Product: **SpanForge Verify**, under the
SpanForge brand; executable/package: `spanforge-verify`. The detailed feature backlog, delivery phases and
acceptance corpus are in the local `implementationplan.md`.
That plan is intentionally ignored by Git; this document preserves the public
direction without claiming its proposed capabilities already exist.

## Promise and first audience

Help developers establish whether an AI system achieved a declared task, followed
the required constraints, handled failure correctly and produced sufficient
independently checkable evidence. Start with coding and automation agents, then
extend adapters and oracles to other AI applications.

The existing CLI runner supplies immutable inputs, process lifecycle management,
assertions, comparison, bundles, HTTP fixtures, scenarios, matrices and repeated
attempts. It now supplies bounded JSON and JSONL agent task/final-response protocols and
hash-pinned post-run executable outcome checks for cooperative targets, with
structured claim-to-check findings, plus bounded reviewed evaluator controls and
oracle health reports. JSONL envelopes retain explicit self-reported event provenance;
they do not establish independently observed actions. It does not yet supply agent tool interception,
automated mutation generation, model dependency recording, model judges or an enforceable security
sandbox. Windows execution is validated; Linux runtime/container acceptance is
still pending.

## Proposed distinctive workflows

1. **Check outcomes against completion claims.** An agent saying tests passed must
   be checked against trusted test execution outside its control. A service action
   must be checked against independently observed persisted state.
2. **Explain permitted and forbidden actions.** Provide an action-policy finding
   linked to observed events, actor scope, approval state and effects. Distinguish
   attempted, denied and executed actions, and disclose unobserved surfaces.
3. **Reproduce the supported failure investigation.** Preserve task, dependencies,
   controlled state and model/tool exchanges in a capsule with an explicit replay
   mode and completeness inventory. No hidden fallback from recorded to live calls.
4. **Show the first observed divergence and reduce the failure.** Present event,
   state and artifact changes and a supported smaller example. Reserve causal claims
   for controlled experiments; probabilistic reductions show uncertainty.
5. **Expose blind spots and release evidence.** Link declared behaviors to actual
   assertions and execution. Missing telemetry, missing oracles and untested behavior
   stay visible even when every existing test passes.

These are product hypotheses, not established claims of novelty. A feature counts
as differentiation when real developers obtain a useful result with less custom
glue, faster investigation or stronger independently checked evidence. We should
measure those outcomes rather than assert that no other tool has a similar feature.

## Public documentation checked

- [Inspect Agent Bridge](https://inspect.aisi.org.uk/agent-bridge.html) documents
  integration of external agents, including sandboxed agents written in different
  languages. Language-neutral agent execution alone is not a distinctive claim.
- [Inspect sandboxing](https://inspect.aisi.org.uk/sandboxing.html) documents sandbox
  execution support. Private working directories alone do not establish containment.
- [Langfuse experiments](https://langfuse.com/docs/evaluation/experiments/experiments-via-sdk)
  documents dataset-driven experiment execution. Datasets/evaluation runs alone are
  not sufficient differentiation.
- [Promptfoo coding-agent red teaming](https://www.promptfoo.dev/docs/red-team/coding-agents/)
  documents coding-agent attack testing. Adding agent adversarial cases alone does
  not establish a unique product.
- [Prooflane](https://prooflane.ai/) publicly describes AI release assurance and
  evidence-based gates. Those supplier descriptions are evidence of positioning
  overlap, not independent verification of all shipped behavior.

This is a scoped documentation review, not an exhaustive market survey or a
benchmark of actual products. The user's goal is to build useful outputs rather
than pursue feature-for-feature competition. Proposed differentiation remains
subject to real integration trials and documented comparison of the relevant flow.

## First proof of value

Implement one local agent adapter and trusted outcome verifier, then exercise a
controlled coding task with a successful fixture agent, false-completion agent and
verifier-tampering attempt. Introduce one real agent integration once containment,
credential handling and resource budgets are available for that target.

Next, demonstrate a simulated automation operation whose apparent success does
not change persisted state. Detect the discrepancy, capture the relevant actions,
and replay recorded dependencies without public network access on a clean supported
host. Preserve a truthful report when the available evidence cannot settle a claim.

Measure time to first reviewed verification, time to understand/replay a failure,
custom integration effort, false positives/negatives on labeled cases and reviewer
agreement. A proposed onboarding target is 15 minutes; it has not been measured.

## Naming

The user selected SpanForge and owns `www.getspanforge.com`. The native runner is
SpanForge Verify; its command is `spanforge-verify`, and its Rust library is
`spanforge_verify`. The old `cliverifyr` command remains a compatibility entry point.
No external package publication or trademark clearance was performed.

The [research-backed gap analysis](agent-verification-gap-research.md) adds evaluator
qualification, environment attribution, trace provenance, dataset integrity and
reviewed incident-to-regression conversion to the roadmap. Integrate the existing
SpanForge SDK through a pinned schema adapter; SDK traces do not replace independent
outcome checks or establish complete observation.

## Evidence boundaries

Results concern declared tests, observed surfaces and supported execution profiles.
They are not universal safety/correctness guarantees. Hashes establish artifact
integrity, not the truth of an agent's claims. Agent self-reported traces do not
establish complete observation. A passing model judge cannot erase a mandatory
action-policy failure. Recorded responses reproduce dependencies, not a live model's
hidden state. Missing required evidence produces an explicit incomplete/inconclusive
result under the reviewed policy.
