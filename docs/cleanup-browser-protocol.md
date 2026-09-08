# Browser protocol execution follow-up

The compatibility suite `browser-audio` now links and executes the
`audio,multiplayer` library test module. The seven existing pure client-protocol
tests retain native test registration and also register with wasm-bindgen-test.
The two existing browser identity cases run alongside the audio cases.
Production protocol and adapter behavior are unchanged.

Acceptance requires actual passing case lines from audio, shared protocol and
identity, including both critical identity cases, and checks that the distinct
passing cases match the runner's total. Fixture regressions cover the selected
link features, missing groups, ignored identity cases and fabricated totals.
The evidence summary records features and grouped passing names.

This verifies shared protocol decisions in Chrome; it does not claim remote
network multiplayer end-to-end execution. See
[the lifecycle runbook](validation/lifecycle-gates.md) for provisioning and
source provenance requirements. Real acceptance must run after committing this
change on a frozen clean checkout, using an already provisioned matched runner,
Chrome and ChromeDriver. Evidence is retained outside the checkout.
