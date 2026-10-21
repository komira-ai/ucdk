# Contributing to ucdk

ucdk is pre-release and not usable yet. Contributions are welcome once the project says so;
until then, open an issue before you start work.

## Sign off every commit

Every commit carries a `Signed-off-by:` trailer in its final paragraph, made with
`git commit -s`. It certifies the [Developer Certificate of Origin](https://developercertificate.org/)
and must carry the email address of the commit's author. You may use your GitHub no-reply
address. `cargo xtask check` refuses a commit without one.

## One command

`cargo xtask check` runs everything the pull-request check runs: format, clippy with warnings
denied, the tests, dependency direction, SPDX headers, the workflow lint, cargo-deny and the
sign-off check. Run `cargo xtask tools` once to install the pinned external tools.

## Rules for changes

- Tests are Rust. A test is a `#[test]` function; no shell scripts or other languages.
- A bug fix comes with a test that failed before the fix.
- Every `.rs` and `.proto` file begins with `// SPDX-License-Identifier: Apache-2.0`.
- Do not include internal or private context in code, commits, branch names or PR text.
- Dependencies come from crates.io only, with a permissive licence (see `deny.toml`).
