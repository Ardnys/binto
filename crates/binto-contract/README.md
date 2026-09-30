# binto-contract

This inner crate contains the shared types of the `binto` project, so that there's only one source of truth for things like `Asset`. This also helps to create developer tools around the asset matching algorithm because it needs constant fixing. `runner` and `insite` agree on what a `RunResult` is with this shared crate.
