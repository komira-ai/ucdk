# Contributing to ucdk

ucdk is pre-release and not usable yet. Contributions are welcome once the project says so;
until then, open an issue before you start work.

## Rules

The rules every change follows are in [AGENTS.md](AGENTS.md), cited by id; this file links to them rather than restating
them, and says how to run the checks that hold them.

## Sign off every commit

Sign off each commit with `git commit -s`, and a merge commit with `git merge --signoff`. What the sign-off must carry, and
what the check compares, is rule P4 in [AGENTS.md](AGENTS.md).

## Checks

Everything is plain cargo. The checks are the Rust crate `xtask`; the pull-request workflow runs
one verb per job, and you can run the same verbs locally.

- `cargo xtask check`: `cargo fmt --check`, clippy with warnings denied, cargo-deny
  (licences, bans, sources), SPDX headers, the workflow run-line rule, the dependency-direction rule (the core crate never depends on the ledger, and a pure crate never depends on an impure one), the `unsafe` rule (the workspace forbids `unsafe`; only `ucdk-capi`, the C ABI crate, lowers it to `deny` in its manifest, and every other manifest inherits the workspace lints), the public-repository lint,
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

The workflow's `run:` lines follow rule B4 in [AGENTS.md](AGENTS.md).

## Context

Do not include internal or private context in code, commits, branch names or PR text: no dates before the project's
public start, home paths, hostnames, addresses or account ids.
