# Contributing to ucdk

ucdk is pre-release and not usable yet. Contributions are welcome once the project says so;
until then, open an issue before you start work.

## Sign off every commit

Every commit carries a `Signed-off-by:` trailer in its final paragraph, made with
`git commit -s`. It certifies the [Developer Certificate of Origin](https://developercertificate.org/)
and must carry the email address of the commit's author. You may use your GitHub no-reply
address (compared case-insensitively). Merge commits inside a pull request need the sign-off too
(`git merge --signoff`), and their messages are linted like any other commit message. The
workflow checks out the pull request's own head, so the commits checked are the ones you pushed.

## Checks

Everything is plain cargo. The checks are the Rust crate `xtask`; the pull-request workflow runs
one verb per job, and you can run the same verbs locally.

- `cargo xtask check`: `cargo fmt --check`, clippy with warnings denied, cargo-deny
  (licences, bans, sources), SPDX headers, the workflow run-line rule, the dependency-direction rule (the planning and provider crates never depend on the ledger or engine, and a pure crate never depends on an impure one), the `unsafe` rule (the workspace forbids `unsafe`; only `ucdk-capi`, the C ABI crate, lowers it to `deny` in its manifest, and every other manifest inherits the workspace lints), the public-repository lint
  and the sign-off check. Each gate prints what it examined (for example `dco: ok (3 commits
  checked)`). In a pull-request run it reads the base commit, title and body from the
  event file the platform provides; locally, `--base <commit-or-ref>` checks the commits since it.
- `cargo xtask coverage`: runs the tests once under `cargo llvm-cov` on a pinned nightly and
  requires every crate, on its own, to cover at least 99% of its lines and 99% of its branches.
  Generated code (under `target/`) and a `main.rs` are not measured; a `main.rs` may only forward
  to its library, in at most three lines after the header. A crate with measured lines and no
  branch points passes the branch floor, which is right only if it truly has none; the run fails
  if no crate in the workspace reports any branch.
- `cargo xtask fetch`: installs the pinned tools (cargo-deny, cargo-llvm-cov). Each is a release
  binary downloaded from a fixed URL and verified against a SHA-256 pin, and checked again before
  each use; `check` fetches only cargo-deny and `coverage` only cargo-llvm-cov, and they do this
  themselves. These checks run on Linux for x86-64.

The workflow's `run:` lines are exactly one `cargo xtask <verb>`; logic goes in `xtask`.

## Rules for changes

- Tests are Rust. A test is a `#[test]` function; no shell scripts or other languages.
- A bug fix comes with a test that failed before the fix.
- Every `.rs` and `.proto` file begins with `// SPDX-License-Identifier: Apache-2.0`.
- Do not include internal or private context in code, commits, branch names or PR text: no dates
  before the project's public start, home paths, hostnames, addresses or account ids.
- Dependencies come from crates.io only, with a permissive licence (see `deny.toml`).
