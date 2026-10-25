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
  (licences, bans, sources), SPDX headers, the workflow run-line rule, the dependency-direction rule (the planning and provider crates never depend on the ledger or engine, and a pure crate never depends on an impure one), the `unsafe` rule (the workspace forbids `unsafe`; only `ucdk-capi`, the C ABI crate, lowers it to `deny` in its manifest, and every other manifest inherits the workspace lints), the public-repository lint,
  the `sdk-fresh` gate (the committed files under `sdk/python/ucdk/generated` and
  `sdk/typescript/src/generated` equal what the generator writes now) and the sign-off check. Each gate prints what it examined (for example `dco: ok (3 commits
  checked)`). In a pull-request run it reads the base commit, title and body from the
  event file the platform provides; locally, `--base <commit-or-ref>` checks the commits since it.
- `cargo xtask coverage`: runs the tests once under `cargo llvm-cov` on a pinned nightly and
  requires every crate, on its own, to cover at least 99% of its lines and 99% of its branches.
  Generated code (under `target/`) and a `main.rs` are not measured; a `main.rs` may only forward
  to its library, in at most three lines after the header. A crate with measured lines and no
  branch points passes the branch floor, which is right only if it truly has none; the run fails
  if no crate in the workspace reports any branch.
- `cargo xtask gen-sdk`: writes the generated SDK sources (`sdk/python/ucdk/generated`,
  `sdk/typescript/src/generated`) from the protos with `ucdk-codegen`. Run it after changing a
  `.proto` file or the generator, and commit the result; the `sdk-fresh` gate of `check` fails on
  a stale file.
- `cargo xtask sdk-test`: builds the shared library (`cargo build -p ucdk-capi`), then runs the
  Python SDK's tests (`sdk/python/tests`) against it with the standard library's `unittest`, and
  the TypeScript SDK's (`sdk/typescript/tests`) with Node's `node:test` on the pinned Node (the
  next entry); the pull-request workflow runs it as the `sdk` job.
- `cargo xtask fetch`: installs the pinned tools (cargo-deny, cargo-llvm-cov, Node). Each is a
  release binary downloaded from a fixed URL and verified against a SHA-256 pin, and checked again
  before each use; `check` fetches only cargo-deny, `coverage` only cargo-llvm-cov and `sdk-test`
  only Node, and they do this themselves. Node is a pinned version because the TypeScript SDK
  binds the library with `node:ffi`, which is in Node 26 and not in earlier versions. These checks run on Linux for x86-64.

The workflow's `run:` lines are exactly one `cargo xtask <verb>`; logic goes in `xtask`.

## Rules for changes

- Tests are Rust. A test is a `#[test]` function; no shell scripts or other languages. The one
  exception is the tests of an SDK in its own language, which test the binding and the generated
  code and nothing else: Python tests live under `sdk/python/tests` and use the standard library's
  `unittest` only; TypeScript tests live under `sdk/typescript/tests` and use Node's built-in
  `node:test` only. No package is installed, nothing is fetched (but the pinned Node), no network
  is used, and both run only through `cargo xtask sdk-test`. A Python or TypeScript test anywhere
  else, or run any other way, is not accepted. The cross-language contract is held from both
  sides: the Rust side writes the golden files in `sdk/wire-golden`
  (`cargo test -p ucdk-api --test sdk_golden -- --ignored write_goldens` rewrites them on
  purpose), and each SDK's tests must reproduce them.
- A bug fix comes with a test that failed before the fix.
- Every `.rs` and `.proto` file begins with `// SPDX-License-Identifier: Apache-2.0`.
- Do not include internal or private context in code, commits, branch names or PR text: no dates
  before the project's public start, home paths, hostnames, addresses or account ids.
- Dependencies come from crates.io only, with a permissive licence (see `deny.toml`).
