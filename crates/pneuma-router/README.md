# pneuma-router

The MIL router as a **pure function**.

```text
fn dispatch(directive: &Directive<Committed>, context: &WorkspaceContext) -> Dispatch
```

No I/O. No async. No state. Given a committed directive and a current
workspace context, the router returns a typed [`Dispatch`] decision:
which executor to route to, what payload, and what to do under various
edge conditions (drift, expired policy, executor-not-permitted,
external-blast under cancellation).

The router is the keystone of MIL's Tier 1 contract validity (see
`MIL-PROJECT.md` Tier 1/2/3 framework). Once `pneuma-router` works,
hand-written `Directive<Committed>` JSON routes correctly end-to-end —
the contract holds.

## Status

v0.2.0. The first slice of the Tier 2 build (Week 1, alongside
`pneuma-core` and `pneuma-acts`).

See [MIL-PROJECT.md](../../../../MIL-PROJECT.md) §11 for the build
order.
