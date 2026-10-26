# ucdk

Pre-release, not usable. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Layout

- `crates/ucdk-core`: the values the other crates share: record paths and keys, hashes, dependency order and the ledger's clock.
- `crates/ucdk-ledger`: the ledger: one document with compare-and-swap writes, a lease and a fence, in memory or in a file.
- `crates/ucdk-release-proto`: the `ucdk.release.v1` messages: a release machine, what a build produces, and what a run reports.
- `crates/ucdk-release`: the release model: machine rules, the set, machine, run and build-inputs hashes, the decisions to publish or stand down, the record codec and the checks of a stored set.
- `crates/ucdk-api`: the `ucdk.api.v1` messages and the one function that shows a ledger.
- `crates/ucdk-capi`: the C ABI, a shared library of five functions.
- `crates/ucdk-codegen`: the generator of the Python and TypeScript SDK sources.
- `crates/ucdk-cli`: the `ucdk` command line: `ucdk ledger show`.
- `sdk/python`, `sdk/typescript`: the SDKs.
- `xtask`: the repository checks, run as `cargo xtask <verb>`.
