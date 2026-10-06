# Spanforge-Core reuse assessment for SpanForge Verify

Reviewed 6 October 2026 against commit
[`545af7a24dbf77f243204ad817a0cc1c37bdef1f`](https://github.com/spanforge/Spanforge-Core/tree/545af7a24dbf77f243204ad817a0cc1c37bdef1f).
The package manifest declares version 1.0.4 and MIT licensing. Relevant source
modules, representative tests and conformance fixtures were inspected statically;
the upstream test suite was not executed. This is an integration assessment,
not a full security audit or certification of the SDK's claims.

The inert source snapshot and file inventory are under the ignored
`target/research/spanforge-core/` directory. No upstream dependencies were installed,
and no product code or implementation-plan completion status changed for this review.

## Assessment: 16 valuable feature groups

Counting related capabilities as groups rather than individual methods, twelve
support the initial verification product and four merit later integration.
"Reuse" means a tested adapter or external SDK component, not copying every Python
implementation into the Rust runner. These are value judgments based on the
approved roadmap; customer usefulness still needs integration trials.

| # | Core capability and source | Value for Verify | Priority / integration boundary |
| --- | --- | --- | --- |
| 1 | [Event envelope and schemas](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/event.py) | Connect agent, tool, evaluation and governance records using stable identifiers. | First release: validate pinned schemas and preserve provenance; do not treat schema validity as behavioral truth. |
| 2 | [Agent runs, steps and nested spans](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/_span.py) | Reconstruct observed execution and locate failed steps. | First release: correlate with runner attempts and independent action receipts. |
| 3 | [Framework/provider integration modules](https://github.com/spanforge/Spanforge-Core/tree/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/integrations) | Reduce instrumentation work for supported Python agents. | First release: start with one validated framework; adapters are instrumentation, not the complete target-execution protocol. |
| 4 | [Local JSONL export](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/export/jsonl.py) and [trace queries](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/_store.py) | Capture an inspectable local evidence stream and retrieve relevant tool/LLM observations. | First release: durable bounded capture, explicit finalization and loss accounting; the optional ring buffer is not an authoritative archive. |
| 5 | [Evaluation scorer protocol](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/eval.py) | Accept existing quality evaluators without inventing another ecosystem. | First release: wrap scorers with qualification, per-check outcomes and explicit errors. |
| 6 | [Token/cost tracking](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/cost.py) | Report success cost, usage and changes between versions. | First release: preserve pricing/provider provenance; add admission control for hard budgets. |
| 7 | [PII handling](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/redact.py) and [secret scanning](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/secrets.py) | Reduce disclosure risk when sharing traces and failure capsules. | First release: keep native literal masking; qualify additional scanners with synthetic controls, including nested fields and attachments. |
| 8 | [HMAC audit chains](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/signing.py) | Check supported integrity properties of captured records. | First release: verify the exact signed scope and key custody; distinguish integrity, completeness and independently observed truth. |
| 9 | [Agent scopes](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/sdk/scope.py) and [RBAC decisions](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/sdk/rbac.py) | Test whether requested actions comply with actor permissions. | First release: consume decisions and enforce required checks at a host-owned gateway; bypassed calls remain uncovered. |
| 10 | [Policy simulation, historical replay and comparison](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/sdk/policy.py) | Preview a policy update against captured observations before activation. | First release: useful additional verification mode; do not label this full agent or model-dependency replay. |
| 11 | [CI gate pipeline and artifacts](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/gate.py) | Connect verification evidence to release decisions and existing pipelines. | First release: export compatible evidence; retain Verify's stricter required-check accounting. |
| 12 | [Testing helpers](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/testing.py), [service mocks](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/testing_mocks.py) and [conformance fixtures](https://github.com/spanforge/Spanforge-Core/tree/545af7a24dbf77f243204ad817a0cc1c37bdef1f/tests/conformance) | Exercise the SDK bridge without live credentials or provider spend. | First release: adapter tests and protocol controls; mocks do not establish real cryptographic validity or realistic stateful service effects. |
| 13 | [Behavior baselines](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/baseline.py) and [drift detection](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/drift.py) | Identify changed latency, usage or behavior worth investigating. | Later: classify drift as a signal; a stable baseline is not a correctness oracle. |
| 14 | [RAG tracing and grounding records](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/sdk/rag.py) | Link answer claims to retrieval observations and supplied grounding assessments. | Later: independently test source relevance, factual support, citation correctness and retrieval access boundaries. |
| 15 | [Prompt registry](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/prompt_registry.py), [model registry](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/model_registry.py) and [lineage](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/sdk/lineage.py) | Associate regressions with declared prompt, model and data versions. | Later: record content hashes and actual resolved dependencies; caller-supplied lineage alone does not prove provenance. |
| 16 | [Human approval workflows](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/workflow.py), [operator views](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/sdk/operator.py) and [evidence bundles](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/sdk/cec.py) | Review findings, approvals and packaged verification evidence. | Later: keep approval-boundary tests in the first release, but defer enterprise workflow integration and compliance presentation. |

## Adaptation requirements found in actual code

These observations apply to the pinned functions, not every Core API or deployment.
They explain why Verify should reuse instrumentation while owning its verdicts.

1. **Scorer errors need explicit outcomes.** `EvalRunner.run` logs scorer exceptions
   and continues, omitting that score. Verify must emit an evaluator-error finding
   for each affected case/check; a missing mandatory result cannot qualify a pass.
   The built-in faithfulness scorer uses token overlap, which is a useful heuristic
   but does not establish factual support. Source: `eval.py:306` and `eval.py:477`.
2. **Missing and duplicate cases need validation.** `RegressionDetector.compare`
   builds key-indexed dictionaries and checks current keys. Removed baseline cases
   are ignored, and duplicate keys collapse. A reviewed test explicitly expects
   removed cases not to be flagged. Verify should preserve that optional diagnostic
   semantics only outside release gates; required case inventory must match.
   [Implementation](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/regression.py#L114),
   [test](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/tests/test_regression.py).
3. **Gate aggregation needs stricter completion rules.** `is_blocking_failure`
   checks only `FAIL`, while aggregation uses that predicate. `ERROR` therefore
   does not itself block through this path. Parallel execution starts only the
   first `max_workers` threads and omits missing results from aggregation. Verify
   needs bounded scheduling for all required checks, explicit not-run/error states,
   and mandatory evidence checks before declaring release readiness.
   [Source](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/gate.py#L168).
4. **Hooks and alerts are not hard enforcement.** Hook callback exceptions are
   suppressed; async hooks may be skipped without an event loop. Budget callbacks
   suppress exceptions too. Good observability behavior must not become a security
   or spending boundary. Enforce mandatory actions and supported budget ceilings
   before dispatch at a protected gateway.
   [Hooks](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/_hooks.py#L279),
   [budget monitor](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/cost.py#L360).
5. **Signing scope must be declared.** `signing.sign` authenticates an event ID,
   payload checksum and predecessor ID. It does not bind all envelope metadata
   such as source, timestamp and actor identity. A required evidence envelope needs
   separately authenticated context and an expected final inventory/anchor; a
   chain alone does not prove that all relevant actions were collected.
   [Source](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/signing.py#L343).
6. **Compliance bundle checks need stronger semantics for release evidence.**
   `SFCECClient.verify_bundle` authenticates the manifest, accepts a stored chain
   validity/structure indication and recognizes timestamp fields. The local
   timestamp is explicitly a stub. These checks must not be promoted to independent
   verification of every archived record or an authenticated timestamp authority.
   Preserve Verify's artifact inventory checks; qualify any signed-bundle adapter
   with payload mutation, missing-file, forged-proof and forged-timestamp controls.
   [Source](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/sdk/cec.py#L917).
7. **Recorded assessments need independent oracles.** `assess_grounding` derives
   its status from caller-supplied claim scores; `generate` in the explain client
   stores supplied explanations/factors. They help organize evidence but do not
   independently discover factual truth or causal reasons.
   [Grounding](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/sdk/rag.py#L417),
   [explanations](https://github.com/spanforge/Spanforge-Core/blob/545af7a24dbf77f243204ad817a0cc1c37bdef1f/src/spanforge/sdk/explain.py#L261).

## Recommended architecture and first integration slice

Keep Core as the optional Python instrumentation/governance component. Keep Verify
as the language-neutral runner, task-contract engine and independent verifier.
Connect them through a versioned, bounded JSONL interface plus an execution and
collection manifest. Do not embed the whole Python SDK in the Rust binary or require
Python for existing native-executable tests.

The collector belongs to a declared producer. Host-owned gateway and outcome checks
produce separate evidence. Every imported record retains its origin and signed
scope. Missing events, sampled events, eviction, crashes and failed flushes appear
in collection status; required findings become inconclusive when evidence is absent.

First slice: a fixture Python agent emits Core agent/tool spans and a false success
claim. Verify captures and validates those records, then runs a protected external
test and reports the discrepancy. Add a correct-agent control, scorer-crash control,
missing-event control and forged-envelope control. Demonstrate the same flow on
Windows laptops and Linux containers before advertising cross-platform support.

Map this into existing workstreams A01/A03/A04/A09/A10/A18/A23, with A21 evaluator
qualification and A24 integrity controls as dependencies. Follow with policy
comparison and reviewed incident-to-regression conversion (A11/A25). This review
does not mark any of those planned integrations as implemented.

Core does not remove the need to build protected independent verifier execution,
stateful action-effect fixtures, complete model/tool dependency capture, offline
agent replay, evaluator qualification, environment attribution or failure reduction.
Caching, broad production exporters, identity-service administration and large
compliance dashboards can remain in Core; they need not expand Verify's first release.
