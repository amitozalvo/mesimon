# Contributing

mesimon is currently a single-author project in its pre-v0.1 phase and is not yet accepting
outside contributions.

## The CLA rule (recorded before it is needed)

The project plans a paid Teams tier alongside the Apache-2.0 core. To keep relicensing rights
intact, **a CLA will be adopted before the first outside pull request is merged.** No outside PR
will be merged before that happens. This rule exists now, in advance, so it is policy rather than
a reaction to any particular contribution.

## Repository boundary

Everything in this repository is Apache-2.0 (`LICENSE`, `NOTICE`). The paid Teams relay lives
in a separate, private repository and is never merged here; its clients (`crates/mesimon-team`,
`crates/mesimon-web`, `web/mesophon`) are part of the Apache core.
