# The mopm release Publisher: the facts, transport and errors, the rig, the scenarios and the changes asked of mopm

Status: design, revision 5. This is the second half of [release-publisher-mopm.md](release-publisher-mopm.md),
split out to keep both files under the project's size bound. It holds sections 3, 10, 11, 12 and 13: the mopm
facts the design relies on, the transport and the error table, the rig, the scenario table and the changes
asked of mopm. Section numbers are global, and sections 1, 2, 4 to 9 and 14 to 17 are in the first file.
Nothing here is implemented yet.

## 3. mopm facts the design relies on

Read from the mopm sources at commit d062f75 of the public mopm repository (`service/api/src` unless a
path says otherwise); not executed. Each bullet names the file and function a reviewer can open. Facts of
mopm#260 (merged, live) are read from its description and the API description it changes, and facts of mopm#261
(in review, not merged) from its description and the specification text it adds; both are marked.

**Package preconditions**

- `POST /groups` requires every member's package to exist and the caller to own it: `requireOwners` in
  `publish/group-service.ts` calls `requireOwner` in `publish/service.ts`, which answers 404
  `package_not_found` or 403 `not_owner`. An account token must belong to a current owner; a CI token must
  name the package's `ci_owner`.
- The claim route is a user-token route (`http/publish-routes.ts`, `claim.ts`), and a CI token cannot call
  it. But the configured-publisher form of the exchange (`exchangeConfigured` in `auth/oidc.ts`, beta stage)
  itself creates a package that does not exist, owned by the CI identity and with no account owner, for the
  publisher. So on the beta stage the first publish of a new name needs no human claim; the exchange is the
  first effect of a release, and it happens before any group. Two consequences. A misspelled but grammatical
  name in a set becomes a permanent package, so the grammar and set checks run before the exchange is
  requested (section 9.2). And group create accepts such a package by its source (`publish/service.ts`
  `requireOwner`); that is read from source, and the manual run confirms it.
- On production a configured identity is permitted only with an `acts_for` account, and the package must
  already be claimed by that account. That is an operator step the publisher cannot do.
- `name_reserved` (403) is an error of the claim route only. The configured exchange creates the package
  with no reserved-name check (`auth/oidc.ts` calls `createPackage` directly), and group create does not
  answer it; the publisher never sees it.
- Group publish is gated per actor, not per stage (`publish/flags.ts`). The `groups_enabled` item is `off`
  (the default), `allow` (a list of actors) or `all`, each with a deny list; beta may also carry a static policy
  of 1 to 8 `ci:gh-repo:<id>` actors. An actor that is not allowed gets 404 `route_not_found` on create
  (`http/publish-routes.ts`), and the message names both causes (a wrong base URL, or no group publish for
  this actor). That is a second operator step beside the CI publishers list. CI publishers load only on beta,
  and on prod only rows with `acts_for` (`auth/ci-publishers.ts`), so on gamma no CI identity can obtain a
  token and none can create a group. A gamma run is a human account's, not a CI grant. This is a setup fact the
  manual run checks.
- Create can also answer 429, 413 `payload_too_large` (a file above 100,000,000 bytes, decimal), 400
  `invalid_request` (including more files than the daily file allowance, a version over 128 characters, an
  empty file, and a group over 20 GB in total), and `limit_exceeded` (per package size quota). A 429
  `rate_limited` carries `Retry-After` as the seconds to the next UTC midnight when the daily counter is
  spent: up to 86400. A 503 `unavailable` answers when a queue is down.
- Which identities may publish is a per-deployment setting of mopm (the list of configured CI publishers).
  The publishing identity must be in the deployment's list; that is an operator request, not a code path.

**Grammars** (`publish/service.ts` `BUILD` and `SAFE_FILENAME`, `version.ts` `GRAMMAR`, `types.ts`
`SUBDIRS_V1`, `auth/tokens.ts`)

- Package name: `^[a-z][a-z0-9_]{1,63}$`.
- Version: exactly `MAJOR.MINOR.PATCH`, three numbers, no leading zeros, nothing else, at most 128 characters
  (`MAX_VERSION_LENGTH`, `version.ts`). No pre-release.
- File name: `mojo-<name>-<version>-<build>.conda`, at most 255 characters, matching
  `^[A-Za-z0-9][A-Za-z0-9._+-]{0,254}$`; the build matches `^mojo[0-9]+(?:\.[0-9]+)*_0$`. A name that fails
  that expression is 400 `invalid_request` (`parseFileFacts`, `publish/service.ts`); one that passes it but is not
  `mojo-<name>-<version>-<build>.conda` with a valid build is 400 `upload_invalid`. The other `invalid_request`
  causes of a member are a size below 1, a malformed `sha256`, a repeated file, more than 500 members and a group
  total above 20 GB.
- Subdirectory: one of `linux-64`, `linux-aarch64`, `noarch`, `osx-arm64` (`SUBDIRS_V1`).
- The archive must be a valid mopm package (`validate/index.ts`). It must hold `lib/mojo/<name>.mojoc`,
  `lib/mojo/<name>.mopm.json` and `info/index.json`. In `info/index.json`, `name` is `mojo-<name>`, `version`
  is the version and `subdir` is the member's subdirectory; its build gives the file name. The manifest `name`
  is the name and its `version`, when it has one, is the version. `manifest_sha256` is the SHA-256 of the raw
  `mopm.json` bytes. A failure is a `rejected` group at validation, not an answer to create. Only Mojo library
  packages are accepted, not arbitrary conda archives.

**Group routes and states**

- Routes: create (`POST /groups`, 201 for created or reopened, 200 for resumed or past that), status
  (`GET /groups/{id}`, which also lazily expires an open group past its deadline), abort (`DELETE /groups/{id}`,
  only an `open` group), commit
  (`POST /groups/{id}/commit`), group yank.
- Create declares every file's `sha256` and `size`; the presigned PUT carries a signed
  `x-amz-checksum-sha256`, an exact length and no bearer token. The upload URL lives at most 300 seconds
  (`publish/config.ts`). Commit declares one `manifest_sha256` per member, the hash of
  `lib/mojo/<name>.mopm.json` inside the archive; it is optional since mopm#261 (below).
- A group has at most one version per package name (the request is refused with "appears twice",
  `group-service.ts`), and several files per member. Two platforms of one `(name, version)` are one member
  with two files; two versions of one name cannot be in one group.
- `create` order (`group-service.ts` `create`): load the group by id; if it exists and is `validating`,
  `committed` or `visible`, answer 200 with every member `skip_identical` and no files, before any
  preflight. If it is `failed`, answer 409 `group_conflict` ("numbers burned"), before any preflight, so a
  re-create of a failed set never reaches `version_exists`. Otherwise open or reopen it, preflight, take a
  slot, supersede, reserve. Create on an ended group reopens it and charges the daily allowance again.
- Preflight and reservation: a `(name, version)` already published, yanked or tombstoned refuses the whole
  create with 409 `version_exists` and nothing is reserved. A pending row held by another live group, the
  open-group cap (5 per actor), the supersede cap (a create may supersede at most 3 of the actor's own open
  groups) and a failed group retried are all 409 `group_conflict`, raised by create only; a refused commit
  is `group_commit_refused`. The response body of every error carries
  `{code, message, hint}`, and a `group_conflict` also carries `reason` (mopm#260, below). A number taken by another publish after the preflight aborts our group on
  the server and answers `group_conflict`; the group is not left open, its rows are free, and the next
  identical create is a fresh attempt, charged again against the daily file allowance (that is the whole
  cost of a conflict mid-reservation; there is nothing to give back, so no change is asked of mopm).
- A create by an actor with a different set that shares a member supersedes the actor's earlier open group
  (`supersede` in `group-service.ts`) and answers 201, unless the request says `on_overlap: refuse`
  (mopm#260, live): then nothing is ended and the answer is `group_conflict` with the reason `overlap_refused`.
  The field is not part of the group id. The create answer lists the groups it ended in `superseded`, absent
  when it ended none and on a call that only continues a group. A registry that predates the option ignores
  it, supersedes, and names nothing.
- **The group id (mopm#261, in review).** It is `grp_` plus the first 32 hex digits of the SHA-256 of the
  compact JSON of `[actor, set]`: the actor in one of the forms `u:<GitHub id>` (an account) or
  `ci:gh-repo:<repository id>` (a configured CI identity), and the set sorted. Member order, file order and
  `on_overlap` are not part of it. The derivation is documented as stable (a change would be a new prefix) and
  has test vectors in mopm's tree, which this crate pins as golden tests. A client computes the id and reads
  the group; there is no lookup by set and no list route.
- **The group read (mopm#261, in review).** `GET /groups/{id}` gives each member `files: [{subdir, filename,
  sha256, size, uploaded}]`, the facts the caller declared, read from the member's reserved version row.
  `uploaded` is present only while the group is `open` (the registry stats the object, so each poll of an open
  group repeats that cost); for any other status it is absent. Members past the member cap and files past 20
  per member are not listed, and a member that is not reserved yet, or whose number another group took after
  this group ended, lists no files. Access is the creator, or an owner of every package (a token that acts for
  an account must also cover every package); a group that is not the caller's is 404 `group_not_found`, never
  403, and the body holds no fact. `getVersion` gains `group_id` when a group published the version (absent,
  not null, otherwise; a pending version is still not found).
- **Commit without a hash (mopm#261, in review).** `manifest_sha256` is optional on the single commit and on
  each member of a group commit. Given: today's checks. Absent: no check, and the version holds the hash the
  registry computes from the archive; a group whose files of one member carry different manifest bytes is then
  `manifest_invalid`. A registry that predates the change answers 400 to an omission, which a client must
  expect during rollout. A repeated group commit must repeat which members gave the hash and what.
- **Conflict reasons (mopm#260, live).** `group_conflict` carries an optional `error.reason` from a closed
  list: `number_held`, `group_failed`, `open_cap`, `supersede_cap`, `takeover_blocked`, `number_taken`,
  `overlap_refused`. A client treats an unknown reason like an absent one. The reason is never rendered into
  the message or hint. Section 6 gives the class of each.
- Group states: `open`, `validating`, `committed`, `visible`, `rejected`, `failed`, `expired`, `aborted`,
  `superseded`. `aborted`, `rejected`, `expired` and `superseded` free the numbers; a number is sealed only
  when a group commits. A group in `validating` cannot be aborted: it proceeds to publish. `failed` burns the
  numbers: the validator applies `group_burn`, which
  turns each pending member into a `tombstoned` record (`state.ts`, `publish/group-validator.ts`).
- Commit (`publish/group-commit.ts`): 202 when `started` is true, and `started` is true both for a fresh
  commit and for an identical repeat while the group is still `validating`; 200 for `committed` and
  `visible`. A commit on an `expired`, `aborted`, `rejected`, `superseded` or `failed` group is 409
  `group_commit_refused`. A commit with an unreserved member is refused ("repeat the create request"),
  and a missing file is 400 `upload_invalid`; both are recoverable by repeating the identical create and
  uploading. DELETE on a group that is not `open` is 409 `group_not_open`; 404 `group_not_found` is also
  what a non-creator receives. All members flip to `committed` in one transaction (`commitGroup` sets
  `committed_at`), so an observer never sees half a set.

**Reads (public; `http/router.ts`). The package and version routes are `cache-control: no-store`; the sparse
index, `config.json` and the repodata objects are `public, max-age=60` (`http/index-reader.ts`,
`publish/indexer.ts`), so an edge-cached reader can be a minute stale after `visible`.**

- `GET /packages/{n}`, `GET /packages/{n}/versions` (paged, limit 1 to 100, cursor),
  `GET /packages/{n}/{v}`, `GET /packages` (search: normalised lowercase prefix of at most 64 characters,
  sorted, cursor), and the sparse index, one anonymous GET of which gives every file's digest. Pending
  versions are invisible to every read route. After the group is `committed` the package API reports the
  members `published`; repodata and index follow, and `visible` comes last.
- Version detail has `state` (`published`, `yanked`, `tombstoned`), `yank_reason`, `tombstone`,
  `files[]` with `sha256` and `size` (and `url` unless the version is tombstoned), and `manifest_sha256`.
  A tombstoned version is returned with 200, never 404: a takedown keeps each file's `sha256`; a version
  burned by a failed group has an empty file list (`tombstoned` in `state.ts` empties the files only of a pending,
  unpublished tombstone; a takedown keeps them).
- `GET /packages/{n}` returns the unpaginated `versions` list, tombstoned versions included. The search
  `latest` is the newest published version only, excluding yanked and tombstoned, and is null when there is
  none.
- 404 comes in kinds: `package_not_found`, `version_not_found` (both genuine absence), `route_not_found`
  (an unknown path, or the index route not configured), `not_in_index` (the sparse index has no entry: index
  lag, or a package whose versions are all burned) and, from a gateway, a body that is not JSON. The index
  answers 502 `index_unavailable` when it cannot be built. Reading the per-version record is exact; the
  index is used for digests only.
- Digests are lowercase 64-hex SHA-256, the same form as `Hash256::as_str`; no conversion is needed.
  Reads are served from a strongly consistent store as deployed (`ConsistentRead`), so the version record
  is uncached and read-after-write exact at the origin; a gateway layer is not known and is covered by the
  manual run. The upload carries a signed checksum that storage verifies and the validator hashes the file
  again at copy time, so the `sha256` of a published file is a digest the registry itself verified at upload.

**Tokens** (`auth/oidc.ts`, `auth/tokens.ts`, `http/publish-routes.ts`)

- The CI exchange mints only `publish`-scope tokens, bound to exact `(name, version)` pairs, 10 minutes,
  and the identity token (`jti`) is single use. The pair list names each package at most once ("names a
  package twice" is refused), so the 1 to 100 limit counts distinct package names.
- There is no `yank` scope for a CI token. Yank by CI works because `authorizeYank` falls back to the
  publish token for the exact `(name, version)`. So every token is publish-capable for its exact pairs, and
  the same token that staged can also commit and yank (until M10).
- The group actor is the account when acting for it, or the configured CI identity otherwise
  (`publish/service.ts`, `ci-publishers.ts`); either way two jobs of one actor share the open-group cap and
  supersede each other.

**Yank**: `PUT /packages/{n}/{v}/yank`, idempotent (a repeat returns the record unchanged and keeps the
first reason), reason 1 to 280 characters, owner only, not gated by read-only mode, needs the index queue
(503 `unavailable` otherwise). A pending or tombstoned version is 409 `invalid_transition`; a missing one
is 404 `version_not_found`. It yanks the whole `(name, version)`, every platform file of it. The channel
specification states that the file stays downloadable at its exact URL after a yank, and version detail keeps
`url` for a yanked version.

**Limits**: 500 members and 20 files per member per group, 100,000,000 bytes per file, 12 hour group
deadline, 100 distinct package names per token.

## 10. Transport and errors

HTTP client: `ureq` (blocking, small dependency tree, matches the synchronous `Publisher` trait) with
rustls. Constraints from `deny.toml`: no `openssl-sys`, no `native-tls`, permissive licences only, no
wildcards. The crypto provider is `ring` (Apache-2.0 AND ISC) and `webpki-roots` is not used because its
licence is not on the allow list. Both are checked by the licence and ban step of `cargo xtask check` in
PR 1; if `ring` is refused the fallback is to ask for an allow-list change, not to add a C TLS stack.
Archive handling (the facts of section 5) uses the pure Rust `zip` (default features off, only the codecs a `.conda`
needs, checked by `cargo deny` in PR 1), `tar` and `ruzstd` crates.

**Trust roots.** `Config.roots` is `Roots::Bundle(path)` and nothing else: one PEM file named by a flag (on a
CI image, the system bundle, passed explicitly). The platform certificate loader is not used, because it can
read certificate-directory variables from the job environment, and the trust anchor of a request that carries
a bearer grant must not depend on the environment. If a platform store is wanted later it is a second
variant that loads a named file or directory and reads no variable. A test sets the usual certificate
variables to a bogus file and shows that nothing changes.

Everything I/O goes through one trait so the publisher is testable without a network and the rig can use an
emulator:

```rust
pub trait Transport {
    fn send(&mut self, request: &Request) -> Result<Response, TransportError>;
}
```

`TransportError` distinguishes "never connected" from "sent, no usable answer". Per the seam, a mutating
call that failed at transport is `Unknown` in both cases; the distinction is kept for diagnostics.

**Transport invariants** (the adapter enforces them and tests each):

- `https` only. The origin of every file URL (the presigned PUT, `files[].url`) must equal the configured
  registry origin or one of the file-host origins listed in `Config`; anything else is refused before the
  request. Redirects are off, for every request. `Authorization` is never sent to a file URL or on a read.
- Proxies are explicit `Config`; the environment proxy settings are disabled in the adapter.
- **Secrets are redacted.** A `Secret` header value type and a URL type that are neither `Debug` nor
  `Display` carry the bearer token and the presigned URL's query signature. Every error message, log line and
  the emulator's request log redact the query string and the `Authorization` value. The bearer is borrowed
  from `Grant::secret()` and the header is built in a zeroizing buffer. A test fails if a canary token or
  signature appears in any `SeamError` message, log line or `Debug` output of a `Request`.

Configuration (a plain struct built by the caller from flags; no environment variables, and no
`MOPM_TOKEN`: the only credential is the grant), with no default registry: registry base URL, file-host
origins, roots bundle path, proxy, request timeout, connect timeout, retry count for idempotent calls, poll
interval, `lease_ttl`, `upload_budget`, `commit_request_timeout`, `poll_deadline`, `abort_timeout`, `margin`,
a `Retry-After` ceiling,
reserving-repeat bound, `newest` page and version caps. `Retry-After` is converted with the seam's
`Backoff::after_seconds` (one tick is `PAUSE_TICK_MS`, one second): rounded up, saturating, and a missing or
unparsable header is a fixed small default, never a retry storm. There is no "ticks per second" setting.

Retries: create and commit are sent once per call (a repeat of create with the identical body is allowed
only for `reserving`, and to refresh expired upload URLs). `PUT` of file bytes and every `GET` retry on
connect failure, timeout, 500, 502, 503, 504 up to the configured count. An upload URL lives at most 300
seconds, and a storage 403 from an expired URL cannot be retried with the same URL: the publisher repeats the
identical create (200, fresh URLs for the files not yet stored) once and retries the `PUT`. To avoid it, a
file's `PUT` starts only if the create answer is younger than half the URL lifetime, else the create is
repeated first. `429` is never retried inside the publisher: it answers `RateLimited` and counts against the
engine's attempt bound. Group create charges a daily counter, so its `Retry-After` can be the seconds to
the next UTC midnight (up to 86400), and the engine waits the stated ticks. Above the configured ceiling
the answer is `Refused(Channel)` ("daily file allowance spent, retry after <time>"; nothing landed), never a
`Pause` of a day; a transcript test pins both sides of the ceiling.

Error mapping (the code is `error.code` of the mopm body; the HTTP status alone is never used to decide).
`Transient` means "failed, and it may have landed" in the seam, so it is used wherever an effect might exist:

| Condition | `SeamError` |
|---|---|
| client-side grant check fails (repository, phase, coverage, malformed envelope) | `GrantRefused(Denied)` |
| client-side or server expiry before commit was sent | `GrantRefused(Expired)` |
| 401 `token_unknown`, `token_revoked`, `token_missing`; authorization refused | `GrantRefused(Denied)` |
| exchange: identity token already used | `GrantRefused(Used)` (never retried with the same identity token) |
| exchange: could not be reached | `GrantRefused(Unreachable)` |
| 429 | `RateLimited(Backoff{ticks})` from `Retry-After` |
| 503 `read_only_mode`; 404 `route_not_found` (create or read; the message names both causes: wrong base URL, or no group publish for this actor); a 429 above the `Retry-After` ceiling; name, version, platform, file name, type, channel or yank reason outside what mopm can hold (client grammar check) | `Refused(Reason::Channel)` |
| 404 `package_not_found` or 403 `not_owner` on create or yank (the package is not created or not owned by the identity); `limit_exceeded`; 413 `payload_too_large` | `Refused(Reason::Channel)` with a message that names the setup step; nothing landed |
| 404 `group_not_found` on the group read | no group (`observe`: not staged; `abort`: section 7.4); any other failure of the group read is `Transient` |
| 404 `package_not_found` or `version_not_found` on a read | `Absent` (observe), `None` (fetch), empty (newest) |
| any other 404, a 404 with an unparsable body, 400 `invalid_name`, `invalid_version`, `invalid_request`, `upload_invalid` on create (a group with more files than the daily allowance also answers `invalid_request`; the code does not tell the two apart and no message is parsed, so the error text carries mopm's message) | read: `Transient`; create: `Permanent` (the client grammar drifted from mopm; fix the check) |
| 409 `version_exists` on create | `Duplicate` (section 5.1's rule for a sealed version) |
| 409 `group_conflict` (create only) | by its `error.reason` (section 6's table); an absent or unknown reason is `Unknown` |
| 409 `group_commit_refused` | by section 7's table, after the group status read |
| group `rejected` during commit | `Permanent` (nothing landed) |
| group `failed` during commit | `Transient` (numbers burned) |
| 409 `invalid_transition` on yank; 400 `invalid_request` on yank; 404 `version_not_found` on yank; a coordinate not held (yank); a well-formed body that violates the response schema before a commit or yank is sent; an invalid archive or archives that disagree on the manifest (until M8) | `Permanent` |
| 503 `unavailable` on create | `Transient` |
| any transport failure, 5xx, unreadable answer, or a well-formed body that violates the response schema, on a mutating call after commit or yank was sent | `Unknown` |
| failure before commit is sent that changed nothing visible | `Transient` |
| any failure of a read, an unparsable or truncated body included | `Transient` |

`Permanent` and `Refused` are never used after a request that may have landed, except where a status read
proves that nothing did (section 7); the tests prove this with an emulator that applies the commit and then
drops the response, and one that fails the group.

## 11. The rig

CI cannot reach a live registry. ucdk's workflows run one `cargo xtask <verb>` and the repository is Rust
only, so a TypeScript mopm cannot be a CI dependency. The rig is therefore:

- **`ucdk-release-mopm-emulator`**, a pure crate implementing `Transport` in process: a model of the mopm
  subset of section 3 with an injected clock, deterministic ids and a request log. It does no I/O, so it can
  be a dev-dependency of the publisher and stay pure. It models what the design depends on, not only the
  happy path:
  - packages and the two identity forms of section 3: the beta configured exchange, which creates a missing
    package unclaimed for the publisher, and the `acts_for` form, which needs the package claimed by the
    account (`package_not_found`, `not_owner` otherwise);
  - groups with every status, including a **validation delay** (the group is `validating` and package reads
    show `Absent` for a configurable number of ticks) so that the lost-answer scenarios run through that
    window, `rejected` (rows freed, `Absent`) and `failed` (every member a tombstone, a burn);
  - `skip_identical` on a repeat create of a past-open group, supersede of an own overlapping open group
    with `on_overlap: refuse` and its `overlap_refused` answer, the `superseded` list under the default, the
    open-group and supersede caps, the `failed` group's `group_conflict`, and every `group_conflict` reason of
    mopm#260;
  - the group read of mopm#261 (declared files, `uploaded` only while open, the documented group id checked
    against its test vectors), a server that ignores `on_overlap` (supersedes and lists nothing, which only the
    overlap probe of the manual run can tell from an honouring one), and a burned
    (tombstoned, no files) record reached by a commit race;
  - 404 kinds (including the index's `not_in_index` and 502), tombstoned version records, signed-checksum PUTs, yank of a whole version, token checks, the
    sparse index (with lag behind the package record), a 429 with `Retry-After`, and the `reserving` answer.
- **The kit's rig hooks.** The seam owners decided one `PublisherRig` shape, once, in the first publisher kit
  part (4g), with defaults that reproduce today's behaviour so the fake's rig does not change. The mopm rig
  implements:
  - `shapes() -> Vec<ChannelShape>`: required. mopm declares one shape, `{stages: true, private: false, group_addressed: true}`: the
    staged shape is the shape. It never declares `private`. The kit asserts `built.stages() == shape.stages` for every publisher it builds, holds a
    `private: false` rig to `public_read_needs_no_grant`, and fails a rig whose `shapes()` is empty. A
    skipped scenario is reported by name with its reason, and is never counted as a pass.
  - `coordinate(label) -> Coordinate`: maps the kit's label to a coordinate mopm can hold: a name of two or
    more characters matching the grammar (the label with a fixed suffix, so a label that is a prefix of
    another stays a prefix, which the listing scenario needs, and distinct labels stay distinct; the kit
    checks both once), the configured channel, type `CONDA` (what `types()` returns), version `1.2.3`, and
    the ucdk platform `linux-x86_64` (the publisher maps it to `linux-64`, section 5). The archives of one
    version that `payload` builds embed identical manifests. The kit's literal
    one-character names, `ArtifactType::File` and the unmapped platform all fail mopm's grammar, so nothing
    commits without this hook.
  - `payload(k, seed) -> Payload { name, bytes }`: a valid `mojo-<name>-<version>-<build>.conda` archive
    with an embedded `lib/mojo/<name>.mopm.json` and content controlled by the seed (seed 0 is "ours", seed
    1 is "theirs"; two seeds give two different, valid archives). The kit computes the expected digest
    from the bytes it gave, never from what the registry reports, so a registry that returned a wrong
    digest cannot pass. mopm does not re-encode, so the SHA-256 of the bytes sent is `files[].sha256`.
  - `siblings(base) -> Vec<Sibling { field, coordinate, placement }>`: for mopm the platform sibling is
    `SameSet` (published with the base) and the version sibling is `OwnSet`; no channel or type sibling
    exists, because mopm cannot hold one.
  - `unrepresentable() -> Vec<Coordinate>`: a foreign channel, a type other than `CONDA`, a name outside
    the grammar, a prerelease version, an unknown platform (including `darwin-arm64`, so a sibling
    expectation is not met by accident), and two coordinates differing only where a `/`
    falls. Every verb on them answers `Refused` and `observe` answers `Refused`, not `Absent`.
  - `seals_versions() -> bool`: `true`. Checked both ways: a rig that says `true` must pass
    `a_sealed_version_refuses_a_later_platform_and_observes_it_absent`, and one that says `false` must
    commit a sibling beside a held coordinate. This is the kit's form of section 5.1.
  - `can_burn() -> bool`: `true` for the emulator, which turns a group `failed` and burns its members.
    Real mopm offers no way to arrange a failed group on demand, so the burned scenario runs on the
    emulator only; the note is in the kit's PR.
- **Helpers ask the shape.** The kit's helpers let a non-staging rig run the ordinary fixtures; the mopm rig
  stages, so every helper runs the real stage and commit. A lost commit may observe `Absent` (before, or
  while the group validates) or `Present` (after); a retry may answer `Committed` or `Duplicate` and must be
  followed by an `observe` that is `Present` with our digest.
- **`MopmRig: PublisherRig`** in the publisher's `tests/` directory, built on the emulator.
  - `publisher(shape)` builds the publisher for the staged shape, and declines any other.
  - `arrange` maps to emulator faults: `HeldByAnother` (another actor publishes the version),
    `StagedByAnother` (another actor's open group holds it), `RaceNextCommit` (the version is taken after
    our reads and before our create: the rig kills our reservation and a rival commits, the answer being
    `Duplicate`), `LoseStagedState` (the reservation is lost and nobody holds the version:
    an error that promises no effect, `observe` `Absent`, a second stage and commit works),
    `CommitVisibleAfter(n)` (the validation delay), `BurnedByFailedGroup`, `LoseAnswerAfterCommit` (apply,
    then drop the response, with the validation delay set), `LoseAnswerBeforeCommit` (drop the request),
    `FailAfterCommit` (apply, answer 503), `CommitSetFailsWhole` (validation rejects, and a second variant
    where the group `failed` and burns), `ListLags` (search omits a version), `FailGroupRead`,
    `FailIndexRead`.
  - `grant` and `foreign_grant` build envelopes; `advance` moves the injected clock; `reads` returns the
    request log mapped to Group and Index steps.
  - **Rig step 1 provisions the identity form under test.** The operator steps are two: the identity's row in
    the deployment's CI publishers, and its entry in the groups allow-list (without it every create is 404
    `route_not_found`). For the configured form there is no claim: the first exchange creates the package,
    and the rig asserts that. For the `acts_for` form the rig claims
    every package of the scenario for the account first. The emulator does both in process; the manual run
    does the first against a beta deployment and the second only with an operator-supplied claim.
- **Fidelity.** An emulator can drift from mopm. Three guards: (1) emulator responses are validated against
  mopm's published API description vendored into the emulator crate with its digest; (2) one
  recorded-transcript test per mopm error code the mapping table uses, and one per `group_conflict` reason; (3) a manually run job (not in CI, not in any pull request) executes the same kit
  script against a non-production mopm deployment with an identity provisioned for that purpose. It starts
  only from a protected environment by manual dispatch, never from a fork pull request, and checks before
  any exchange that the registry's index configuration, or its health route, names a non-production stage (both
  carry `stage` already; the guard is ours, M5), refusing with nothing sent if it does not. `Config`
  has no default registry. No production registry is ever a rig target. The manual run's checklist confirms what the source says:
  that group create accepts a package the configured exchange created (owned by the CI identity); the uncached strongly
  consistent version record; `Present` at `committed` with repodata lagging; all members of a 20-member
  group reading `Present` in the same observe round; that `files[].sha256` equals the SHA-256 of the
  downloaded bytes; that a takedown keeps its digest; the group id derivation against the documented vectors (mopm#261), and the id the create answer names against the computed one; yank retention at the
  channel host; the overlap probe (a second, overlapping set under `on_overlap: refuse` answers
  `overlap_refused` and ends the first group, which is the proof of decision 20); that a repeat create offers no
  upload URL for a file the registry holds (7.1); and the real token exchange and its 10 minute life (section 9.3). The kit's new checks of
  `commit_unit()`, `conditional_put()` and `grant_limit()` (4g) and the rig assert the values section 4
  states.
- **Transport adapter tests.** The `ureq` adapter is covered by a few tests against a `std::net::TcpListener`
  stub (timeouts, partial bodies, 429 with `Retry-After`, redirects refused, a foreign file origin refused).
  Everything above it runs on the emulator.
- Coverage: the 99% line and branch bar applies to the publisher and to the emulator separately.

Rows of section 12 marked "emulator" are proven against this document's reading of mopm, not against mopm.
No row is proven against real mopm until the manual run of PR 7 repeats it.

## 12. Scenario table

The rows follow the order of the kit's scenario list (the number is the position in that list), keyed by
name. Each cell carries one status:

- **met**: the design meets it, on the shape and with the hooks of section 11, and the emulator will prove it;
- **met after Mx**: met once the named mopm change lands (section 13); the shape in question is not built or
  the row cannot pass before then;
- **skipped**: the shape the publisher declares has no such state, so the kit skips it by shape and reports it
  (a skip is not a pass);
- **GAP**: not met, with the reason.

Every row presupposes the documented group id (mopm#261): it is how the publisher addresses a group at all.
"met (files)" rows also rest on the group read's list of declared files (mopm#261), which is in review. The
conflict reasons and `on_overlap` (mopm#260) are live. "Emulator" marks a row whose evidence is only the
in-process emulator even after the manual run.

| # | Scenario | Status |
|---|---|---|
| 1 | observe_absent_is_not_an_error | met: `Absent` only for a parsed 404 `package_not_found` or `version_not_found`; a missing platform of a held version is `Absent`; the kit's sibling checks use `siblings()` (platform `SameSet`, version `OwnSet`) |
| 2 | stage_then_observe_returns_staged_with_the_same_digest | met (files): create and PUT, then group read plus package read; the declared digest storage verified |
| 3 | a_staged_coordinate_is_not_readable_by_a_stranger | met: mopm hides pending rows from every read; `fetch` follows the record's url only |
| 4 | a_stage_grant_observes_the_staged_state_it_made | met (files): group read with the stage envelope |
| 5 | a_commit_grant_observes_it_too | met (files): same token class, commit envelope |
| 6 | commit_makes_the_staged_coordinates_readable_with_the_same_digest | met: commit to `visible`, `Present` with `files[].sha256` |
| 7 | stage_of_present_same_is_duplicate_and_changes_nothing | met: `version_exists` preflight, nothing reserved; platform siblings go `SameSet` |
| 8 | commit_over_present_different_leaves_old_bytes | met: stage gives `Duplicate` first; a commit on a dead group gives `Duplicate` when the version is held |
| 9 | abort_gives_staged_state_back_and_never_touches_a_committed_coordinate | met: abort table of 7.4; `committed` gives `Ok` without effect |
| 10 | a_set_commit_makes_every_coordinate_readable_or_none | met: one group; `rejected` leaves `Absent`; `failed` leaves `Burned`, never `Absent`; emulator models both |
| 11 | a_channel_that_does_not_stage_commits_with_the_bytes | skipped: mopm stages |
| 12 | no_digest_exposed_is_refused | met on the emulator only: real mopm exposes `sha256` for every file, so a 200 without it is `Permanent` ("malformed") and only a broken emulator produces it |
| 13 | yank_keeps_bytes_fetchable | pending kit 4g (the `yank_unit()` branch is decided; not a mopm gap): the draft scenario says a yank reaches only the coordinate it names, which mopm's `Version` unit cannot pass; with the branch it is met (siblings named and covered: both yanked; one omitted: `GrantRefused(Denied)`, nothing changes). Retention at the channel host is the manual run's. |
| 14 | yanked_coordinate_cannot_be_committed_again | met: `version_exists` gives `Duplicate`, observe reads `Yanked` |
| 15 | lost_answer_is_resolvable_by_observe | met (files): `Staged` or `Present`, or `Absent` while the group validates |
| 16 | list_may_lag_but_observe_is_exact | GAP until M3 is widened (question 2): the listing helper compares the whole `Held`, stamp and exceptional mark included; observe is exact from the uncached record |
| 17 | public_read_needs_no_grant | met: all reads unauthenticated; a write without a grant is `Denied` |
| 18 | a_grant_for_another_repository_sends_nothing | met: envelope check on every verb including reads; request log empty; proves the envelope logic only |
| 19 | a_grant_of_the_wrong_phase_is_denied_and_nothing_happens | met client side (proves the envelope logic only: a stage and a commit token are both mopm `publish` tokens; the channel and type siblings the draft check used are `unrepresentable()` instead); server enforcement after M10 |
| 20 | an_expired_grant_is_refused_and_nothing_happens | met: injected clock, checked on every verb; 401 `token_expired`; the real exchange is the manual run's |
| 21 | newest_lists_every_held_coordinate_of_the_name | GAP until M3 is widened: the scenario compares digest, stamp and exceptional mark of every held coordinate; the digests, the yanked ones, the empty-package case, the all-or-nothing read and the cap are met |
| 22 | observe_returns_the_stamp_it_was_given | GAP: M3 as taken stores `source_commit` only; the build number and the exceptional mark have no home (question 2) |
| 23 | a_granted_observe_sees_a_coordinate_committed_by_another_writer | met: public read |
| 24 | a_private_observe_reads_the_group_then_the_index | skipped: mopm declares no private shape (G-private) |
| 25 | a_coordinate_committed_in_another_group_reads_as_present_never_absent | met: `Present` with mopm's digest |
| 26 | a_failed_group_read_or_index_read_fails_the_observe_and_is_not_absent | met: a bare 404 or a 404 with another code is `Transient` on the package read and on the group read (only `group_not_found` is "no group") |
| 27 | observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set | met (files): the group id computed from the declared set |
| 28 | observe_with_an_empty_set_cannot_see_staged_state_and_still_sees_committed_coordinates | met: package read only |
| 29 | a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit | met: read is `Grant::none()`; a Read envelope carrying a token is refused; a write is refused client side |
| 30 | commit_is_atomic_when_the_coordinate_appears_between_observe_and_commit | met: realized as a lost reservation plus a rival commit, answer `Duplicate` only |
| 31 | transient_after_commit_is_observed_before_retry | met: `Unknown`, never `Permanent` |
| 32 | duplicate_over_other_bytes_leaves_the_old_bytes | met: a stage of other bytes over one's own staged coordinate is `overlap_refused`, never superseded (mopm#260), answered `Duplicate`; platform siblings go `SameSet` |
| 33 | a_second_commit_of_the_same_bytes_is_duplicate | met: body status `committed` or `visible` at entry; on a `validating` group the retry may give `Committed`, which the kit accepts |
| 34 | a_stage_over_ones_own_staged_coordinate_is_duplicate | met: identical set, group read, `Duplicate`; other bytes, the overlap reason |
| 35 | a_commit_carries_exactly_what_was_staged | GAP: the bytes are met (declared digests read from the group, the registry computes the manifest hash), but the kit asserts the "another stamp" refusal and the stamp has no home (row 22) |
| 36 | a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was | met: preflight before reserve; a conflict mid-reservation ends the group server side |
| 37 | a_set_that_names_a_coordinate_twice_is_refused | met: client check, no I/O |
| 38 | a_stage_or_commit_grant_reads_no_staged_state_but_through_observe | met: `fetch`, `newest`, `list` are public reads and carry no token |
| 39 | a_channel_that_does_not_stage_creates_a_coordinate_once | skipped: mopm stages |

Scenarios the seam owners' answers add to the kit's last two parts, and their status for mopm:

| Scenario (name as decided) | Status |
|---|---|
| a_burned_coordinate_is_neither_absent_nor_present_nor_committable | met on the emulator only (`can_burn`): `observe` `Burned`, stage and commit `Duplicate` (a tombstoned record is checked before the missing-coordinate rule of 5.1), `fetch` `None`, `newest` omits it |
| a_sealed_version_refuses_a_later_platform_and_observes_it_absent | met (`seals_versions`): `Refused(Channel)`, `Absent` |
| a_coordinate_the_channel_cannot_hold_is_refused_before_anything_is_sent | met: every verb `Refused`, `observe` `Refused` |
| a_retry_of_a_commit_that_is_becoming_readable_is_committed_or_duplicate_and_never_a_second_write | met: one landed write, then `Present` with our digest |
| a_commit_of_staged_state_that_vanished_is_not_committed_and_a_restage_works | met: `Permanent` on the status read's evidence (7.2), then a new `stage` works |
| a_registry_that_echoes_the_declared_digest_fails_readback | met: `fetch` returns the bytes served and the engine hashes them |
| the yank-unit `Version` branch | see row 13 |

Counted from the 39 rows above: 26 met (including row 12, emulator only, and row 19, client side only), 5 more
met on the group read's file listing (rows 2, 4, 5, 15, 27), 3 skipped because mopm stages (rows 11, 24, 39),
1 pending the kit (row 13) and 4 GAP (rows 16, 21, 22, 35). Row 13 is pending the kit, not a mopm gap, and rows
16, 21 and 35 rest on the draft kit, so re-count them when kit part 4g lands; rows 16, 21, 22 and 35 are the real
mopm gaps (the stamp). "Met" is not a pass: no row is a pass against real mopm until the manual run, and a skip
is reported by the kit and is not a pass.

## 13. Changes asked of mopm, the maintainers' verdicts and the agreed order

Every verdict was checked against the mopm sources at commit d062f75. All taken changes are additive and
wire-compatible. Service changes reach a deployment through its own promotion path; this repository changes
none. "Before" says what the publisher does until the change lands, which is what ships first.

**State and order.** M13 and M14 landed in mopm#260 and are live in production (release 0.0.76). M1, M6 and M8
are in mopm#261, in review. Still to come: the client-side re-exchange (M2, no deploy) and the documentation
items (M11 and the token-form notes), then M10, then M3. Question 2 of section 16 asks to move a widened M3
ahead of M10, because four kit rows depend on it. M16 is new, with no verdict yet.

| Id | Verdict | What mopm does | Before it lands the publisher | After |
|---|---|---|---|---|
| M13 | taken; LIVE (mopm#260) | keeps 409 `group_conflict` (raised by create only) and adds a stable `error.reason` from a closed documented list of seven: `number_held`, `group_failed`, `open_cap`, `supersede_cap`, `takeover_blocked`, `number_taken`, `overlap_refused`; the reason is never in the message or hint | landed | classifies by reason (section 6 table); an absent or unknown reason is `Unknown`, and the engine goes to its read-back, **never `Duplicate`**; no message is parsed |
| M14 | taken; LIVE (mopm#260) | `POST /groups` takes `on_overlap: supersede` or `refuse` (default `supersede`); under `refuse` an overlap with the caller's own open group is `group_conflict` / `overlap_refused` naming the own group ids, and nothing is ended; the create answer lists the groups it superseded (absent when none and on a continuing call); the field is not part of the group id | landed | sends `refuse` always; a `superseded` list under `refuse` is a contradiction and is refused; there is no positive marker of honour in an answer (M16), so a registry's level is proven by the manual run's overlap probe (section 6) |
| M1 | taken, modified; in review (mopm#261) | `GET /groups/{id}` lists each member's `files [{subdir, filename, sha256, size, uploaded}]` (`uploaded` only while open); the group id derivation (actor plus declared set) is documented with test vectors; the publisher computes the id and reads it (404 is no group). Never create to inspect | the staged shape is designed and proven on the emulator against this documented contract; PR 2 starts when the PR has merged; a real registry needs it deployed (PR 7) | `observe` of a declared set, `stage` and `abort` use the group read |
| M6 | taken; in review (mopm#261) | `getVersion` emits `group_id` when the version came from a group | decides whose bytes a version holds by digest alone (the seam's rule) | may tell its own group from a foreign one, and tell a burn from a takedown by the group's status |
| M8 | taken; in review (mopm#261) | `manifest_sha256` optional on commit: present is today's check, absent is server computed; with none declared, files of one member with different manifest bytes are `manifest_invalid` | sends no hash; the archive is read before any I/O only to refuse early what the registry would reject (a group spends the allowance per attempt); a registry that predates it answers 400 to the omission, which the commit table maps (7.2) | the same: no manifest hash code exists to delete |
| M10 | take, raised to stage 3 | a new token scope `stage` (create, upload, get, abort); commit and yank require `publish` and `yank` | phase discipline is client side only (section 9) | the grant source asks for the scope of the phase; the server enforces the table |
| M3 | take, modified (stage 4) | one typed immutable field `source_commit` (a git sha) on commit, returned by `getVersion`; no free-form map | stores no stamp: `observe` reports `stamp: None` (truthful, and the engine already reads it as unstamped, which makes a supersede decision indeterminate); refuses an item whose `Held` carries the exceptional mark (`Refused(Channel)`, nothing sent) rather than record it as ordinary | stores and returns the source commit. The build number and the exceptional mark still have no home: rows 16, 21, 22 stay GAP (question 2) |
| M2 | take (no deploy) | client side only: on a 401 `token_expired` re-request the identity JWT and exchange again; no longer token, no refresh route | does not rely on it; the call is bounded below the token and lease (section 9.3) | the grant source may re-exchange; using it inside a call needs a seam addition (question 3) |
| M9 | take | group scenarios and goldens in the existing wire harness, not a separate transcript system | hand-written transcripts plus the manual run | goldens generate guard 2 of section 11 |
| M5 | take as docs (no config change) | none: `index/config.json` (`stage`, `api_url`, `channel_url`, `read_only`) and the health route (`stage`, `read_only`) already carry the stage; nothing says whether group publish is enabled for an actor, and the 404 message covers that | the manual run reads the stage and refuses a registry whose stage is not non-production (ours to enforce) | same |
| M11 | docs only | state that `getVersion` is `no-store`, and the token forms and `ci_owner` of group create | proven by the manual run at the channel host | the API description says it |
| M7 | already exists | one anonymous GET of the sparse index gives every file digest | `newest` uses it (section 4), falling back to the record for a version the index lags on | same |
| M15 | already exists | CI publishers are configured per deployment (beta, or prod with `acts_for`; gamma has none), and group create is allowed per actor, not per stage; the identity needs a row in the deployment's list and an entry in the groups allow-list, which the operators add on request | both must be requested before the manual run; rig step 1 documents them | same |
| M4 | rejected | none; the cost of a conflict mid-reservation is documented instead: a number taken after the preflight aborts the group and the next identical create is charged against the daily file allowance | states that cost (section 3) | same |
| M12 | rejected | none; on `version_exists` the public `getVersion` `files[]` set is the comparison | compares that set (section 5.1) | same |
| GAP-S | rejected | none; the publisher decides the full platform set before the first group | refuses a later platform (`Refused(Channel)`), and a multi-platform release needs the engine and machine change of section 5.1 | same |
| G-private | out of scope for mopm | none; the guard is ours | refuses a PRIVATE repository at construction and guards on the identity token's `repository_visibility` claim (section 8) | same |
| M16 | proposed (no verdict yet) | the create answer echoes the `on_overlap` value it applied, so a client can tell a registry that honours the option from one that ignores it | the manual run's overlap probe is the only proof (section 6) | the publisher refuses a create answer that does not echo `refuse` |
