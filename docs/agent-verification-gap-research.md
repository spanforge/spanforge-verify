# SpanForge Verify: evidence-led product opportunities

Research reviewed 6 October 2026. This is a primary-source documentation review,
not an exhaustive market benchmark. Problems below are documented; demand for
our particular solution and differentiation remain hypotheses to validate.
The broader capabilities described here remain planned. Initial development now
includes cooperative JSON agent tasks, pinned post-run outcome checks and
structured claim findings, JSONL self-reported event envelopes, reviewed evaluator
controls and oracle health reports;
see the README for their supported scope and limits.

## What the evidence supports

| Source | Documented observation | Implication for our product |
| --- | --- | --- |
| [Anthropic: agent evaluation design](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents), January 2026 | Completion text and actual environment outcomes can differ. Incorrect graders, ambiguous tasks and overly strict checks can invalidate evaluation results. | Independently check outcomes and qualify the test itself before trusting scores. |
| [Anthropic: infrastructure noise](https://www.anthropic.com/engineering/infrastructure-noise), February 2026 | Controlled coding evaluations changed with resource allocation and enforcement. Infrastructure errors and resource-dependent task success are different effects. | Capture resource contracts and separate observed environment faults from target failures. |
| [METR: MALT](https://metr.org/blog/2025-10-14-malt-dataset-of-natural-and-prompted-behaviors/), October 2025 | Manually reviewed transcripts include natural and prompted evaluation-integrity threats, with benign controls. Monitoring also needs validation data. | Protect evaluators and test detection against benign controls, not just attack examples. |
| [LangChain: State of Agent Engineering](https://www.langchain.com/state-of-agent-engineering) | The vendor survey reports substantially wider observability adoption than offline evaluation adoption. | Converting traces into reviewed regression tests may reduce workflow friction. The sample is self-selected; it does not establish market-wide demand. |

## Existing capabilities we must acknowledge

[Inspect Agent Bridge](https://inspect.aisi.org.uk/agent-bridge.html) already supports
external agent integration. [Langfuse experiments](https://langfuse.com/docs/evaluation/experiments/experiments-via-sdk)
already support dataset-based evaluations. [Promptfoo coding-agent red teaming](https://www.promptfoo.dev/docs/red-team/coding-agents/)
explicitly covers verifier sabotage, boundary evidence, protected checks and
host-side verification. Independent checks, agent adapters, traces, sandboxing and
red teaming are therefore not defensible claims of unique invention.

Our opportunity is a coherent, local verification workflow with qualified tests,
declared observation limits, inspectable outcome evidence and reproducible failure
investigation. Documentation alone cannot establish that competitors lack this
whole workflow. Compare the actual developer journey before claiming superiority.

## Prioritized features and concrete outputs

| Priority / plan | Feature | Output developers receive | Acceptance test |
| --- | --- | --- | --- |
| P0 / A01–A03 | Task contracts and independent outcome verification | Claim/outcome discrepancy with trusted check receipts and persisted-state evidence | A false-completion agent fails even when its prose and self-reported tests say success; a valid alternate solution passes. |
| P0 / A21 | Evaluator qualification | Oracle health report: reference acceptance, negative-control rejection, mutation escape list, ambiguous checks | A known-correct solution passes; a no-op and seeded broken solutions fail; an always-pass evaluator cannot qualify a release. |
| P0–P1 / A04, A07, A23 | Observation and containment manifest | Per-surface observed, enforced, self-reported, missing and unsupported capabilities | Removing telemetry makes affected claims unknown; a forged SDK event cannot become an independent action receipt. |
| P1 / A05–A08 | Stateful action fixtures and explicit dependency replay | Action/effect witness and portable failure capsule with a replayability inventory | A successful HTTP reply with no state change fails the declared task; a missing recorded exchange never silently makes a live call. |
| P1 / A22 | Environment and dependency attribution | Failure ledger with resource limits, environment probe results and supported comparison conditions | A killed container, unavailable tool and wrong agent output remain distinct; unequal resource profiles prevent an unqualified capability ranking. |
| P1 / A14, A24 | Evaluator and dataset integrity | Protected-check receipts, holdout exposure ledger and contamination warnings | Editing agent-visible tests cannot change host-owned verdicts; cross-attempt canaries and exposed private cases are detected or flagged. |
| P2 / A23, A25 | SpanForge trace-to-regression workflow | Sanitized, reviewable task draft from an incident, with missing evidence and a qualified verifier | Import omits secrets, retains provenance and requires review; a regression independently fails before entering the release suite. |
| P2 / A10–A12, A17 | Evidence-based change and release decisions | Paired outcome/action/cost change report; PASS, FAIL or INCONCLUSIVE linked to evidence | Missing mandatory checks remain inconclusive; a critical violation cannot be averaged away. |

## SpanForge SDK integration

The existing [SpanForge standard](https://www.getspanforge.com/standard) supplies
an event-schema starting point. Verify should consume a pinned, tested schema
through an adapter rather than introduce a competing trace format. Confirm the
actual SDK release and payload contract using its repository and fixtures before
shipping interoperability; website descriptions alone are insufficient.

An SDK trace is evidence from its declared producer. Importing it must not imply
complete observation, verified actor identity or independent enforcement. Preserve
producer identity, schema version, omitted events and collection scope; correlate
with gateway receipts and trusted external state checks when available.

## First customer workflow and proof of value

Start with coding and automation agents whose outcomes can be checked against a
repository or synthetic service. Deliver one successful run, one false-completion
run and one evaluator-failure run with distinct verdicts. Then demonstrate an
incident converted into a reviewed test and replayed offline on a clean host.

Measure onboarding time, custom integration code, time to diagnose and reproduce,
false positive/negative rates on labeled controls, and reviewer agreement. Compare
the same tasks against an existing evaluation setup, recording versions and effort.
Proposed targets: a first reviewed check within 15 minutes, no missed seeded
critical violations in the acceptance corpus, and useful investigation without
reading the entire raw trace. These are validation targets, not achieved results.

Prioritize this workflow over broad provider lists, another dashboard, general
agent hosting or universal safety claims. Discovery interviews and real integration
trials remain pending; no customer outreach or external publishing was performed.
