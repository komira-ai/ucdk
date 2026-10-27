# The mopm release Publisher

Status: design, revision 3. Nothing in this document is implemented yet. Where it says "the publisher does X"
it describes what the crate will do when the PRs in section 15 land. The document is in two files to stay
under the size bound: this one holds sections 1 to 10 and 14 to 16 (the design); sections 11 to 13 (the rig,
the scenario table and the changes asked of mopm) are in
[release-publisher-mopm-verification.md](release-publisher-mopm-verification.md). Section numbers are global.

Revision 3 folds in the review of revision 2 and revision 2 folded in the review of revision 1, the answers of the seam owners (the seam changes of
ucdk#42, now on `main`), and the verdicts of the mopm maintainers on the gaps. What changed is listed in
section 16.

## 1. Summary

ucdk's first real release channel is a mopm registry. This document designs `ucdk-release-mopm`, a Rust
crate that implements `ucdk_release_seams::Publisher` against mopm's HTTP API (the group publish path and
the public read path) and is held to the publisher kit (`run_publisher_kit::<R: PublisherRig>`, 39
scenarios, nine contract points, and the scenarios the seam owners added for the kit's last two parts).

Three facts shape everything:

1. mopm has a server-side staged set: a *group*. It is open and invisible, it is create-if-absent per
   `(name, version)`, and its commit is all-or-none. That is a good fit for `CommitUnit::Set`.
2. mopm cannot be asked what a group holds. `GET /groups/{id}` returns member states, never digests or the
   declared file list, and the only call that finds a group from its declared set is `POST /groups`, which
   is a write (it reopens dead groups, charges counters and supersedes overlapping groups). `observe` must be
   a read. So `Staged` cannot be observed today. The maintainers will extend `GET /groups/{id}` (gap M1,
   section 13), and the staged shape is built behind it.
3. mopm seals a `(name, version)` once a group commits (and burns it if the group fails). A later set cannot
   add a platform to a version. So a release for several platforms must declare its whole platform set before
   the first group that can commit (section 5.1), and every platform archive of one version must agree on one
   manifest. The maintainers rejected a change here; the engine and the release machine must adapt.

The publisher therefore ships in two shapes:

- **One-call shape** (`stages() == false`). `commit` creates the group, uploads, commits and waits for
  visibility in one call. It works against mopm as deployed today. `stage` and `abort` are never called.
- **Staged shape** (`stages() == true`). `stage` creates the group and uploads, `commit` commits it. It needs
  gaps M1, M13 and M14 and is built behind them.

The shape is a construction parameter, not a runtime guess, and there is one code path for each verb (no
parallel API): the staged shape adds the stage and abort verbs and changes what `commit` sends; everything
else is shared.

mopm is a public registry for Mojo library packages. It has no private visibility, no per-phase scopes, no
place to store a build stamp today, and it accepts only valid Mojo library archives of a fixed name grammar.
Section 13 lists every gap with the maintainers' verdict, the agreed order, and what the publisher does
until each lands (refuse, or degrade with the degradation named). Section 12 says, scenario by scenario,
which are met, which are met after a named mopm change, which are skipped by the shape the publisher
declares, and which are a gap.

Three things in this document are stated as limits of the evidence, not as facts about the product:

- Everything in section 3 was read from the mopm sources, not executed.
- The in-process emulator (section 11) copies this document's reading of mopm. A scenario that passes on it
  is evidence about the publisher, not about mopm, until the manual run of PR 7 repeats it against a real
  deployment.
- The kit parts for the publisher (4g, 4h) have not landed. Their hooks are those the seam owners decided
  (section 11); a row that depends on a hook says so.

## 2. Goals and non-goals

Goals

- Implement every `Publisher` method with the semantics in `publisher.rs`, and pass the kit for the shapes
  the publisher declares.
- Never decide an effect on a lagging read. `observe` reads the authoritative per-version record; `list` is
  used for reports only. No verdict of "published" rests on a digest the registry reports: the read-back
  fetches the bytes and the engine hashes them (section 8).
- A mutating call whose transport failed answers `Unknown`; a call after which an effect may have landed
  never answers `Permanent` or `Refused`.
- Refuse, at construction or preflight, what mopm cannot honour, instead of faking it. Refuse before any I/O
  whatever mopm would answer with a 400, so that no request that can never succeed is ever sent or retried.
- Client-side enforcement of grant repository, phase, coverage and expiry, so a refused call sends nothing.
- Bound every call below the lease and the token it runs under, checked at construction (section 9.3).
- All timeouts, retry counts, page caps and the registry location are configuration passed in by the
  caller, never environment variables. Trust roots are one explicit bundle file named by a flag.

Non-goals

- The OIDC exchange itself. It belongs to the grant source (section 9 gives the envelope both sides share,
  and PR 4 builds the mopm grant source that produces it).
- Private repositories. mopm has none; the publisher refuses a PRIVATE repository (section 8, G-private).
- Claiming a package name. The publisher never claims one (section 3 says how a package comes to exist).
- Reaching a live registry from CI. CI runs the kit against an in-process emulator (section 11).
- Changing the seam, or mopm, from this repository. Seam changes are decided by the seam owners; mopm
  changes are requests (section 13).

## 3. mopm facts the design relies on

Read from the mopm sources at commit d062f75 of the public mopm repository (`service/api/src` unless a
path says otherwise); not executed. Each bullet names the file and function a reviewer can open.

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
- Group publish is off by default on every stage, and is also gated per actor: an allow-list of actors
  (`publish/flags.ts`: the groups switch, or on beta a policy naming the CI identity). When the switch is off
  or the actor is not on the list, create answers 404 `route_not_found` (`http/publish-routes.ts`). That is
  a second operator step beside the CI publishers list, and the 404 message names both causes (a wrong base
  URL, or no group publish for this actor).
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
- Version: exactly `MAJOR.MINOR.PATCH`, three numbers, no leading zeros, nothing else. No pre-release.
- File name: `mojo-<name>-<version>-<build>.conda`, at most 255 characters, matching
  `^[A-Za-z0-9][A-Za-z0-9._+-]{0,254}$`; the build matches `^mojo[0-9]+(?:\.[0-9]+)*_0$`. A file whose name
  is not of this form is 400 `upload_invalid`.
- Subdirectory: one of `linux-64`, `linux-aarch64`, `noarch`, `osx-arm64` (`SUBDIRS_V1`).
- The archive must be a valid mopm package: the validator checks the layout and the embedded
  `lib/mojo/<name>.mopm.json` (`validate/layout.ts`, `validate/index.ts`). Only Mojo library packages are
  accepted, not arbitrary conda archives.

**Group routes and states**

- Routes: create (`POST /groups`, 201 for created or reopened, 200 for resumed or past that), status
  (`GET /groups/{id}`, which also lazily expires an open group past its deadline), abort (`DELETE /groups/{id}`,
  only an `open` group), commit
  (`POST /groups/{id}/commit`), group yank.
- Create declares every file's `sha256` and `size`; the presigned PUT carries a signed
  `x-amz-checksum-sha256`, an exact length and no bearer token. The upload URL lives at most 300 seconds
  (`publish/config.ts`). Commit declares one `manifest_sha256` per member, the hash of
  `lib/mojo/<name>.mopm.json` inside the archive.
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
  only `{code, message, hint}`. A number taken by another publish after the preflight aborts our group on
  the server and answers `group_conflict`; the group is not left open, its rows are free, and the next
  identical create is a fresh attempt, charged again against the daily file allowance (that is the whole
  cost of a conflict mid-reservation; there is nothing to give back, so no change is asked of mopm).
- A create by an actor with a different set that shares a member supersedes the actor's earlier open group
  (`supersede` in `group-service.ts`) and answers 201.
- The group id is a function of the actor and the declared set. Its derivation is not a documented interface
  today (it will become one, M1).
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
index is `public, max-age=60` (`http/index-reader.ts`), so a cached index can be a minute stale on top of the
build lag.**

- `GET /packages/{n}`, `GET /packages/{n}/versions` (paged, limit 1 to 100, cursor),
  `GET /packages/{n}/{v}`, `GET /packages` (search: normalised lowercase prefix of at most 64 characters,
  sorted, cursor), and the sparse index, one anonymous GET of which gives every file's digest. Pending
  versions are invisible to every read route. After the group is `committed` the package API reports the
  members `published`; repodata and index follow, and `visible` comes last.
- Version detail has `state` (`published`, `yanked`, `tombstoned`), `yank_reason`, `tombstone`,
  `files[]` with `sha256` and `size` (and `url` unless the version is tombstoned), and `manifest_sha256`.
  A tombstoned version is returned with 200, never 404: a takedown keeps each file's `sha256`; a version
  burned by a failed group has an empty file list (`state.ts` empties the files only of a pending, unpublished
  tombstone; a takedown keeps them). The manual run confirms it.
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

## 4. The mapping

`Publisher` capability getters, per shape:

| Getter | One-call shape | Staged shape | Why |
|---|---|---|---|
| `types()` | `[CONDA]` | same | a `.conda` file is a conda artifact; `CONDA` names an identity (the SHA-256 of the file's bytes, which is mopm's `files[].sha256`) since ucdk#42. A coordinate of any other type is `Refused(Channel)`. |
| `stages()` | `false` | `true` | see section 1 |
| `commit_unit()` | `Set` | `Set` | a group commits all or none |
| `conditional_put()` | `true` | `true` | create refuses an existing version (`version_exists`), commit is compare-and-set on group status |
| `grant_limit()` | `100` | `100` | counted in coordinates, as the seam defines it; mopm's limit counts distinct names, so no set of 100 coordinates can exceed it. It refuses at most four times too many legal sets (four subdirectories). |
| `yank_unit()` | `Version` | `Version` | mopm yanks a `(name, version)`: every platform file (section 7) |

The binding limit for a large tree is the 100 distinct names per token, however it is counted: a release of
more than 100 packages must be several Publish actions (a set is one action). That is a release-machine
decision, and the two limits agree on it.

Method mapping. "Public" means no token. "Token" means the grant's secret as bearer, attached only to the
calls that say so and never to a read or a file URL.

| Method | mopm call | Grant used | Error mapping (summary; full table in section 10) | Idempotency and atomicity argument |
|---|---|---|---|---|
| `observe(g, k, set)` | One-call shape: `GET /packages/{n}/{v}` (public). Staged shape: the group read of the group id recomputed from the actor and `set` (M1, token), then the same package read. | none needed. A grant that is presented is checked for repository and expiry on every verb, and for coverage where the verb names coordinates (section 9). | A 404 whose parsed body code is `package_not_found` or `version_not_found` gives `Absent`. Any other 404, an unparsable 404, and every read failure is `Transient` (`Refused(Channel)` for `route_not_found`), never `Absent`. A tombstone is `Burned` or `Yanked` (section 8). An expired grant that is presented is `GrantRefused(Expired)`, which `SeamError::observe_failure` treats as `INDETERMINATE` even at preflight; the publisher's own expiry check on a read therefore applies only to a grant that was presented. | A read. Exactness rests on the version record being uncached and strongly consistent (section 3, proven against a deployment by the manual run). Never calls `list`, search or create. Reports the registry's own digest; no verdict rests on it alone (section 8). |
| `stage(g, items)` (staged shape) | the group read (M1) first: held and uploaded gives `Duplicate`; else `POST /groups` with `on_overlap: refuse` (M14), repeated with the identical body while `reserving`, then `PUT` each file the response says is not held, with the returned headers and no token | stage grant | 409 `version_exists` gives `Duplicate` (a burned coordinate too: the engine's observe reads `Burned`); every `group_conflict` is classified by section 6 (`Unknown` until M13) | The group id is a function of actor and declared set, so a repeat of the identical set resumes the same group. A different set that shares a member is refused, never superseded (M14); a create answer that shows it was superseded anyway is "not honoured" (section 6). |
| `commit(g, items)` one-call | create (with `on_overlap: refuse`, whose honouring is checked, section 6), upload, `POST /groups/{id}/commit`, poll `GET /groups/{id}` until `visible`; the create answer is classified by section 7 (own group already `validating`, `committed` or `visible` skips upload and commit) | commit grant (covers create, upload, abort of own group, commit, poll) | see sections 7 and 10 | one group is one atomic commit; `version_exists` is the create-if-absent |
| `commit(g, items)` staged | `POST /groups/{id}/commit` with the manifest hashes, poll to `visible`. The answer is decided by the group status in the body, not the HTTP status. | commit grant | body status already `committed` or `visible` gives `Duplicate`; `validating` is polled and gives `Committed` at `visible` | an identical commit body is idempotent server-side; a different body is 409 `group_commit_refused` |
| `abort(g, ks)` (staged shape) | `DELETE /groups/{id}`; on 409 or 404, `GET /groups/{id}` | stage grant | section 7 table | DELETE is idempotent only for `aborted`; the follow-up read makes the rest idempotent |
| `yank(g, k, reason)` | read the version record (`k` must be held in it, else `Permanent`), check the grant covers every held sibling, then `PUT /packages/{n}/{v}/yank` with `{yanked: true, reason}` | commit grant covering `k` and every held sibling of its version | see section 7 | server-side no-op on repeat. Coarser than a coordinate: `yank_unit() == Version`, so the engine names and covers every sibling (section 7). |
| `fetch(g, k)` | `GET /packages/{n}/{v}`, then `GET` the file `url` from that record for `k`'s subdirectory | none | 404 version, no such file, a tombstoned version (no url) give `None` | Returns the bytes as served, and judges nothing: the engine hashes them against the set's digest. Never builds a URL itself, so staged bytes are never returned. The URL's origin is checked against configuration (section 10). |
| `newest(g, ch, name)` | `GET /packages/{n}` (versions, newest first, yanked and takedown-tombstoned included), one sparse-index read for the digests, and the per-version record for any version the index does not show yet | none | see rules below | A read of committed state; compares nothing. |
| `list(g, scope)` | `GET /packages?sort=name` paged, stop once names pass the prefix, then the latest version's record per package | none | read failures are `Transient` | Reports only; omitting older versions, yanked-only and tombstoned-only packages (search `latest` is null for them) is the permitted lag. |

`newest` rules. No package (404 `package_not_found`) and a claimed package with no published version give an
empty list, not an error. The sparse index is a cache of up to 60 seconds on top of the build lag, so it is
read only for digests; a version whose digest matters is read from its own record, which is exact. An index
404 `not_in_index` is lag (or an all-burned package) and falls back to the records; a 502 `index_unavailable`
fails the call (`Transient`). A version burned by a failed group holds no `Held` and is skipped (the seam's
rule: a burned coordinate is not returned); a takedown tombstone with a digest is listed as `Yanked`. The
sparse index lags the package record, so a version in the record and not yet in the index is read from its
record, never omitted. If any one of the reads fails, the whole call fails (`Transient`), and a partial list
is never returned. A versions list over the configured cap is `Permanent`, never silently truncated.

## 5. Coordinates and keys

A ucdk `Coordinate` is `{channel, type, name, version, platform}`. mopm's identity is `(name, version)`, with
files `mojo-<name>-<version>-<build>.conda` under a subdirectory. The client check uses mopm's exact
grammars (section 3), and everything outside them is `Refused(Reason::Channel)` before any I/O, for
`observe`, `fetch`, `newest`, `stage`, `commit`, `yank` and `abort` alike. A name that mopm would answer with
400 `invalid_name` can never be read as `Transient` and retried:

- `channel` must equal the channel the publisher was constructed for, else `Refused(Reason::Channel)`.
- `type` must be `CONDA`, else `Refused(Reason::Channel)`.
- `name` must match `^[a-z][a-z0-9_]{1,63}$`. So `Demo` and `my-pkg` are refused locally.
- `version` must match exactly three dot-separated numbers without leading zeros. So `1.0.0-rc1`, `1.0` and
  `01.2.3` are refused locally.
- `platform` is ucdk's `<os>-<cpu>` or `any`, mapped to a mopm subdirectory by one table in the crate and
  nothing else: `any` is `noarch`, `linux-x86_64` is `linux-64`, `linux-aarch64` is `linux-aarch64`,
  `darwin-aarch64` is `osx-arm64`. The table is a bijection (a test checks both directions, so a subdirectory
  read from a record maps back to exactly one ucdk platform), and a test compares its right-hand side with
  `SUBDIRS_V1` of section 3. Any other platform is `Refused(Reason::Channel)`. The macOS spelling follows the in-tree convention
  (`darwin-aarch64`, as in the machine tests of `ucdk-release`); `darwin-arm64`, which the build-system kit
  uses as an unmapped platform, stays unmapped and the rig lists it in `unrepresentable()`. The archive's
  embedded `info/index.json` subdirectory must equal the mapped subdirectory (mopm answers `subdir_mismatch`),
  so the check reads the archive as well as the blob name.
- The file is the `Blob` whose `name` is `mojo-<name>-<version>-<build>.conda` with the coordinate's name and
  version, a build matching `^mojo[0-9]+(?:\.[0-9]+)*_0$`, and at most 255 characters. A mismatch is
  `Refused(Reason::Channel)` before anything is sent. Until M8, a blob that is not a valid package archive
  (it has no embedded manifest) is `Permanent` before any I/O. Also checked before any I/O, each a
  `Permanent` or `Refused` so that no daily allowance is spent on a group mopm would reject: a version over
  128 characters, an empty file, a group over 20 GB in total, and (until M8) that every archive of one member
  embeds the same `lib/mojo/<name>.mopm.json` bytes and agrees on build and native library (section 5.1).

Consequences, each enforced in code:

- **One file per coordinate.** All files of one `(name, version)` share one build (the build is in the file
  name) and mopm refuses a duplicate file, so a version holds at most one file per subdirectory. A record that
  shows two files for one subdirectory is a broken response: `Permanent` ("malformed").
- **Byte-exact.** The grammars above make name and version injective with mopm's own spelling (one version
  has one spelling). Each is percent-encoded as one path segment, and the name and version echoed by mopm
  must equal the request byte for byte, else `Permanent`. The publisher never normalizes. Inputs outside the
  grammar are not mapped to something similar; they are refused as unrepresentable.
- **Injective.** The publisher never joins fields with `/` or any separator to compare or key anything.
  Sets are checked with derived field-wise equality on `Coordinate`, and member grouping uses a
  `BTreeMap<(name, version), ...>` of owned fields. Grant coverage compares the five fields of each
  coordinate against a length-framed list in the grant envelope (section 9). Tests use coordinates that
  differ only where a `/` falls and expect the publisher to refuse them at the grammar check, and, for the
  emulator, to treat distinct keys as distinct.
- **Set-wide checks happen before any I/O.** A set naming one coordinate twice is `Permanent` for `stage`,
  `commit` and `observe` (scenario 37). A set naming two versions of one package name is
  `Refused(Reason::Channel)` ("one version per name"), because neither a group nor an exchanged token can
  cover it. Two coordinates of one `(name, version)` with different platforms form one group member with two
  files. A test proves that an invalid-grammar version never produces a request.

### 5.1 A version is sealed by its first group: multi-platform releases

mopm seals `(name, version)` when a group commits (and burns it if the group fails). A later set that names
another platform of a published version cannot be created: it is `version_exists`, forever. The maintainers rejected a
change to this (gap GAP-S in section 13). It is ucdk's to handle, and today it is not handled:

- `release.md` gives one PUBLISH stage per platform, so the platforms of one release are published by
  separate Publish actions, each its own set. Against mopm the first platform publishes and **every later
  platform of the same version is refused forever**. mopm cannot serve a multi-platform release under that
  stage model. A release for one platform, or for `any` alone, is not affected.
- The publisher's part is small and total. After a 409 `version_exists` it reads the version record. First,
  a tombstoned record (a burned number: no files) gives `Duplicate`, so that the engine's observe reads
  `Burned` and refuses `PUBLISH_BURNED`; the missing-coordinate rule below applies only to a published or
  yanked record. Then every requested coordinate present with the same digest gives `Duplicate`; a requested
  coordinate present with another digest gives `Duplicate` (old bytes stand); any requested coordinate
  missing gives `Refused(Reason::Channel)` ("the version is sealed with another file set"). `observe` of the missing
  coordinate answers `Absent`, so the engine cannot loop observe-`Absent`, commit-`Duplicate`. `Refused`
  promises that nothing happened, which is true. The rig declares `seals_versions() == true` and the kit
  holds it to this answer in both directions (section 11).
- **What the engine and the release machine must do.** The platform set of a release has to be fixed before
  the first group of any version: all platforms of one `(name, version)` in one group, therefore one Publish
  action, therefore one set. Concretely: (1) the machine declares the full platform set of the release, and
  the engine admits a Publish action only when every platform of that set has a sealed build; (2) the
  Publish action's set is the join of the per-platform release sets, published once; (3) a machine for a
  channel that seals versions whose publish is split per platform is refused at admission, before any build,
  with `Reason::Channel`, never half-published. This changes `release.md`'s stage model (one platform per
  stage, `machine.stage_platform`, and question 10 there); the publisher cannot supply it. Until it lands, a
  mopm channel is usable for single-platform releases only, and the doc of the channel says so.
- **One manifest per member.** mopm checks every file of a member against one declared `manifest_sha256` and
  rejects the whole group unless all of them agree on build, dependencies and native library (id and soname).
  Joining platforms into one group therefore works only for a package whose per-platform archives embed the
  same `lib/mojo/<name>.mopm.json` bytes and the same native soname. A package whose archives differ (a
  native library named differently on linux and macOS) cannot share a member: it is single-platform on mopm
  until mopm accepts a manifest per file (a new request, not in section 13's verdicts). The publisher checks
  agreement client side (section 5) and answers `Permanent` before any I/O.
- A version that was published with a partial platform set stays partial. The recovery is a new version,
  not a retry. The same holds for a version burned by a failed group (section 7).

Which of (1) to (3) the engine takes, and whether (3) alone is enough for the first release, is a decision
for the engine owners (question 1 in section 16). The recommendation is (3) now, as a cheap refusal that
makes the failure impossible to miss, and (1) with (2) as the fix.

## 6. Staging is a mopm group

Applies to the staged shape.

- `stage` computes the members (one per `(name, version)`, files sorted), recomputes the group id from the
  actor and the declared set (the derivation M1 documents and pins by test), and reads the group
  (`GET /groups/{id}`, M1). 404 is "no group". A group that is open with every file of the set uploaded is
  `Duplicate`, with no create sent.   Otherwise it sends `POST /groups` with `on_overlap: refuse` (M14) and
  repeats the identical request while the answer is `reserving` (bounded by configuration).
- **Detecting a server that ignores `on_overlap`.** An old mopm ignores the field and supersedes silently,
  which is the hazard M14 exists for. M14 therefore also puts the superseded groups in the create response
  (and a reason on a refusal). The staged shape checks every create response: one that lists a superseded
  group, or that lacks the new response field, means "refuse not honoured", and the shape answers
  `Refused(Channel)` ("registry does not honour on_overlap") for that stage and for every later one of the
  same publisher, with the group aborted when it is the actor's own. The same check runs in the one-call
  shape once the field is sent. The emulator has a server that ignores the field and a case for each.
- The publisher never uses create to inspect a group: create on an ended group reopens it and charges the
  allowance, and an unrefused create can supersede another open group.
- States an observer sees while the group is `open`: no read route shows a pending version, so a stranger
  reads `Absent`, `fetch` returns `None`, and `newest` and `list` omit it. That is the seam's "staged is not
  readable by a stranger", provided by mopm for reads of the version record. Unreleased bytes are not secret
  on mopm: the quarantine object key is derived from values a party knowing the actor and the declared files
  can compute, and this document makes no claim about the storage access policy.
- Create answers:
  - 201: the group was created or reopened as the next attempt, and files are uploaded. `Staged`.
  - 200 with some member not `skip_identical`: the group exists and is open; missing files are uploaded
    (same declared digests, signed checksum). `Staged` if bytes were added, `Duplicate` if every file was
    already held.
  - 200 with every member `skip_identical` and no files (the group is `validating`, `committed` or
    `visible`): nothing to upload. `Duplicate`.
- **`group_conflict` is one code for unrelated conditions, and the code alone is never enough.** Until M13
  gives it a stable `error.reason` from a closed documented enum, the publisher parses no message (a wording
  change in a mopm release would silently change the class) and answers every `group_conflict` (raised by
  create only) `Unknown`. The engine goes to its read-back, which polls `Publish.polls` times (default 3, one
  tick apart); the action is `INDETERMINATE` when those are spent, and `Duplicate` is never answered for a
  conflict. After M13 the reason decides, by this table (the
  reason names are mopm's; the classes are what the publisher needs):

| Reason class | Staged shape | One-call shape |
|---|---|---|
| a member held by another live group (pending, or "no header") | `Duplicate` (staged by another) | `InFlight` |
| the actor's own open group holds it, other bytes (an overlap refused, M14) | `Duplicate` (the first staged bytes stand) | `InFlight` |
| open-group cap (5 per actor) | `Refused(Channel)` ("abort an open group") | same |
| supersede cap | `Refused(Channel)` | same |
| a failed group's numbers are burned | `Transient` ("burned; publish a new version"); the engine's observe reads `Burned` | same |
| a number was taken (our group aborted by the server) | `Duplicate` if the version record now shows the coordinate held, else `Transient` once | `InFlight` if held by a pending group, else as staged |
| our own group is in another state and shares a version | `Refused(Channel)` | same |
| any reason not in the table | `Unknown` | same |

  A caller over its open-group cap is never told `Duplicate`, so the engine cannot loop. Each row is pinned
  by an emulator case and, once mopm publishes transcripts (M9), by a golden. A burned set re-created by
  the same actor reaches `group_conflict` before `version_exists`, so it is this table's burned row, never
  `Duplicate`; a burned number reached by a different set is `version_exists`, `Duplicate`, and the
  engine's observe reads `Burned` (the seam refuses it as `PUBLISH_BURNED`, never retried).
- **Same-actor supersede (M14).** Without the `on_overlap` field, a create of a set different from an earlier
  open set of the same actor, sharing a member, answers 201 and discards the earlier group's staged bytes.
  The publisher cannot see that without M1, so it does not build the staged shape before M1 and M14, and
  sends `refuse` always once they exist, relying on it only as the previous bullet says. In the one-call shape the earlier group is open only while its own
  call runs, and the engine's per-channel lease serialises publishes to one channel; the exposure is an
  exceptional run and a normal run of the same actor on overlapping members (section 9.1).
- Create is sent once per attempt and never blindly retried after an `Unknown`; the engine observes first.
  Two exceptions, each bounded: the identical create while `reserving`, and a re-create to refresh upload
  URLs (section 10).
- `observe` must not call create. Create reopens dead groups, charges daily file counters and can
  supersede another open group sharing a version. A "read" with those effects would break the seam. Hence M1.
- Staged state expires (12 hour deadline, lazily). A staged coordinate whose group expired reads `Absent`;
  the engine's observe-then-stage then creates a fresh attempt. Abandoned open groups use one of the 5
  open-group slots of the actor until then, which is an operational note for the staged shape. Runbook: a
  refusal for the open-group or supersede cap ends the wave `REFUSED`; the operator aborts the stale groups
  and reruns.

## 7. Commit, lost answers, abort and yank

### Commit

Common steps, in order:

1. Validate the call with no I/O: grammar and set checks (section 5), the package archive and its
   `manifest_sha256` (until M8), grant checks (section 9), the call's time budget (section 9.3), member
   grouping.
2. One-call shape: create (the identical request repeated while the answer is `reserving`, bounded by
   configuration, because commit needs every member reserved), then classify its answer.
   - 201, or 200 on an open group: upload every file the response lists, then go to step 3.
   - 200 with every member `skip_identical` on an own group whose body status is `validating`: this is our
     own commit still in flight (the group id is a function of actor and set). Skip upload and commit and go
     to step 4, which answers `Committed` at `visible`.
   - 200 with every member `skip_identical` on an own group already `committed` or `visible`: nothing
     changed by this call. `Duplicate`; the engine's following `observe` says whether the held digest is
     ours (the seam leaves the digest of a `Duplicate` to `observe`).
   - 409: `version_exists` is section 5.1's rule; `group_conflict` is section 6's table. Failure here changes
     nothing visible; the publisher best-effort aborts its own group.
3. `POST /groups/{id}/commit`, sent once. From the moment its first byte is written, every failure
   (connect, timeout, 5xx, lost response, unreadable body) answers `Unknown`, per the seam. A 409
   `group_commit_refused` is followed by a read of the group status and the version record, by this table:

   | Group status read after a refused commit | Answer |
   |---|---|
   | `committed`, `visible` | `Duplicate` |
   | `validating` | poll (step 4) |
   | `expired`, `aborted`, `superseded` (our group changed under us; one-call shape) | `Transient`: the identical call re-creates it and succeeds, and `observe` settles what landed |
   | the same, in the staged shape, and the version record does not hold the coordinate | `Permanent`, because the status read is the evidence that nothing landed and the recovery is a new `stage`, a different call |
   | `rejected` | `Permanent` (numbers freed, nothing landed) |
   | `failed` | `Transient` ("numbers burned"); never `Permanent`, because the burn is an effect |
   | the status read itself fails | `Unknown` |

   A refusal that says a member is unreserved, or a 400 `upload_invalid` for a missing file, is recoverable
   by repeating the identical create and uploading, and gives `Transient`.
4. Poll `GET /groups/{id}` (token) every configured interval until `visible` or the call's deadline.
   `rejected` gives `Permanent`. `failed` gives `Transient` with a message that the numbers are burned.
   The deadline, a 401 on a poll (token expired) or a read failure gives `Unknown`.
5. `Committed` is returned only at `visible`, so every read path of every member shows the set.

All-or-none: the group is one unit. `rejected` before the copy phase publishes nothing and leaves the
coordinates `Absent`. A `failed` group, after the copy phase, burns the numbers: channel files may sit at
exact URLs, and every member is a tombstoned record. That is **not** `Absent`, and it is not `Permanent`
either, because the burn is an effect that landed. The answer is `Transient`, the engine observes, `observe`
reads `Burned` (section 8), and the action is `REFUSED` with `PUBLISH_BURNED`: a new version is needed. A
later `commit` of that coordinate by another set answers `Duplicate` (`version_exists`), and the same
observe reads `Burned`. Visibility across subdirectories and index files is per read path, not
simultaneous; `Committed` waits for `visible`, and `observe` reads the package record, which flips at
`committed`. A reader of repodata can lag `observe`. That is why the smoke action polls.

Atomic against a coordinate appearing between observe and commit: create's `version_exists` is the
compare-and-set. The answer is `Duplicate` and nothing is created. In the staged shape the reservation makes
the race impossible after `stage`; the race exists only during `stage`.

Lost answers: `commit` answering `Unknown` or `Transient` is resolved by `observe`. The group can then be in
three places. `visible`: `observe` shows `Present` with the registry's digest, and a retried commit answers
`Duplicate`. `committed` or `validating`: after `POST /commit` is accepted the group is `validating` and
package reads still show `Absent` until `commitGroup` sets `committed_at`; a retry's create answers 200
`skip_identical` and the publisher goes to polling (step 2), so the retry answers `Committed` or
`Duplicate`, and the engine's observe and read-back tell whether the bytes are ours. Never started:
`observe` shows `Absent`, and the retry re-sends the identical create (the same group, resumed) and commit.
In every case the publisher does not decide whose bytes they are. A test holds the `validating` window with
`observe` giving `Absent`, and a second test applies, drops the answer, lets the group reach `visible`,
retries and expects `Duplicate` and then `Present`.

### Abort (staged shape)

`abort(g, ks)` recomputes the group id (M1) and sends `DELETE`:

| mopm answer | Publisher answer |
|---|---|
| 200 or already `aborted` | `Ok` |
| 404 `group_not_found` | `Ok` only if the group read (or the recorded group id) agrees the group is gone; otherwise `Unknown` (a non-creator and an uncovered token also get 404) |
| 409 `group_not_open`, then `GET`: `expired`, `rejected`, `superseded`, `aborted` | `Ok` |
| `committed`, `visible` | `Ok`, touches nothing (only yank can undo) |
| `validating` | `InFlight`: a validating group cannot be aborted and proceeds to publish; a later observe settles it |
| `failed` | `Ok` (nothing to give back; numbers are burned) |
| 401, 403 | `GrantRefused` |

In the one-call shape the engine never calls `abort`; the publisher still gives back its own half-built
group inside a failed `commit` (step 2), and a failed cleanup is expected and is resolved by resume.

### Yank

mopm's unit is the version, the seam's kit says a yank reaches only what it names, and the seam owners
resolved the two by making the unit explicit: `yank_unit() == YankUnit::Version` for mopm, so a call
withdraws every held coordinate that agrees with `k` in channel, type, name and version, whatever its
platform. A publisher never widens a yank silently. The rules:

- **The engine** (the yank verb, a later PR) calls `newest(ch, name)`, collects the held siblings of the
  version, and refuses (`REFUSED`, listing them) unless every one is named and covered by the commit grant it
  exchanges. It writes `yanking` per coordinate, calls `yank` once, reads back `Yanked` for each, and writes
  `yanked` per coordinate. Explicit beats implicit: the operator chooses the reach.
- **The publisher** keeps its own check as defence in depth, because a publisher that depended on the engine
  alone would exceed its grant on a buggy caller. Before anything is sent it reads the version record. If
  `k` is not held in it (no file for `k`'s subdirectory), the answer is `Permanent` ("not held"; nothing to
  yank, nothing sent, the same call fails again, as the conforming fake answers). If the record holds a
  coordinate the grant does not cover, the answer is `GrantRefused(Denied)` and nothing is sent. The reason
  must be 1 to 280 characters, else `Refused(Reason::Channel)` (a grammar failure).
- A pending or tombstoned version (409 `invalid_transition`) and a missing one (404 `version_not_found`) are
  `Permanent` (nothing happened, the same call fails again). A transport failure after the request was
  written is `Unknown`.
- Bytes stay fetchable: the channel specification says so, and the version record keeps the `url`; the manual
  run proves it for the real channel host.

A yanked coordinate can never be committed again: the version row persists, so create answers
`version_exists`, which maps to `Duplicate`, and the engine's observe reads `Yanked`, refused as
`PUBLISH_YANKED`.

Yank by group (`PUT /groups/{id}/yank`) is not used. When M6 lands, the version record names its group, which
lets the publisher say whether a version is its own; yank by group stays unused, because the unit is the same.

## 8. observe on a private registry, declared set and empty set

mopm has no private visibility, so the publisher declares itself public-only and refuses to be constructed
for a `PRIVATE` repository, for an `UNSPECIFIED` visibility, and, for any write capability, for an anonymous
push identity (a federated identity is required). Visibility is not a mopm concept to add (G-private is out
of scope there); the guard is ours. The mopm grant source reads the `repository_visibility` claim of the
identity token it exchanges and refuses (`GrantRefused(Denied)`, nothing exchanged) unless the channel
declares `PUBLIC` and the claim says the source repository is public, so a private repository's build cannot
be published to a public registry by a misconfigured machine. The claim is read, not verified, here (mopm
verifies the token); the guard catches our own mistakes. What the kit's private scenarios mean for mopm:

- Reads need no grant and carry no token. A grant that is presented is checked (repository, expiry, and
  coverage where the verb names coordinates) and is otherwise used only for the group read in the staged
  shape.
- **Version states.** `published` is `Present`. `yanked` is `Yanked(Held{sha256})`. A `tombstoned` record is
  never `Absent` and never `Present`. One with an empty file list is a number burned by a failed group:
  `Burned`, a presence of its own since ucdk#42 (it holds no bytes, so there is no digest to compare, and the
  engine refuses it as `PUBLISH_BURNED`, never retried). One that kept a file `sha256` (a takedown) is
  `Yanked(Held{sha256})`, which the engine refuses as `PUBLISH_YANKED`. By the source a burned member holds
  no files (section 3), so the two do not meet; the manual run confirms it. If a deployment ever showed a
  burned member with a digest, it would read as a takedown, carry no stamp, and make the supersede decision
  of that channel `INDETERMINATE`: a known gap until M6 names the group, whose `failed` status tells the two
  apart. `fetch` of a tombstoned version is `None` (the record has no url). `newest` follows section 4. A
  200 whose body lacks `sha256` for a published or yanked file is `Permanent` ("malformed"); the registry's
  own response schema requires it, so only a broken emulator or gateway produces it. The publisher never
  reports `Yanked` with an empty digest to stand for a burn: an empty digest in a `Held` already means "no
  digest exposed", and a yanked build with no stamp makes a channel's supersede decision indeterminate.
- **Declared set.** Staged shape: `observe(g, k, set)` with a non-empty `set` reads the group (M1) and the
  package record and combines them: package record found wins; otherwise a group read that finds `k` staged
  with all files `uploaded` gives `Staged(held)`, the digest being the declared one that storage verified at
  upload; otherwise `Absent`. Both reads use the same grant and no other call is made. A non-empty `set`
  lacking `k` is `Refused(Reason::Channel)`.
- **Empty set.** Only the package read is made. Staged state is never reported without a set, because the
  group is addressed by the declared set.
- **Another group's coordinate.** Any version published by any group or single publish reads `Present` with
  the digest mopm holds, which differs from ours when the bytes differ. It is never `Absent`.
- **Failed reads.** A failed group read or package read fails the whole observe (`Transient`, or `Unknown` if
  the answer was lost). It is never read as "no group" or "not committed".
- One-call shape: only the package read exists; a non-empty `set` is checked for `k` and then ignored.
- A pending version held by another live group reads `Absent` (mopm hides pending rows). The following
  `commit` or `stage` answers as in section 6.
- **What a failed observe means to the engine.** `SeamError::observe_failure` (ucdk#42) decides: in the
  preflight observe, before any write, a `Refused(r)` keeps its reason `r`, a `Permanent` is `REFUSED
  CHANNEL`, a denied grant is `REFUSED CREDENTIAL`; after an effect that may have landed (the observe after
  a `Duplicate` or a commit answer, the read-back), every failure is `INDETERMINATE`, because it says
  nothing of the effect before it. So a coordinate this channel cannot hold is refused cleanly at
  preflight, and the same refusal after a commit is never mistaken for "nothing happened".
- **Digest contract.** The digest `observe` reports is the record's `sha256`: the one the registry itself
  verified over the stored bytes at upload (section 3), never one only the writer declared. That is the
  contract the seam now states. The publisher uses it for the preflight and the commit phase only. The
  read-back (`CHANNEL_READBACK`, a smoke action, the read-back of a publish) `fetch`es the bytes through
  the same grant rules and the engine hashes them; the publisher's `fetch` returns the bytes as served and
  checks nothing against the record. A mopm that echoed the writer's declared digest would fail the
  read-back, and a kit scenario holds a registry that does.

## 9. Grants

`Grant` is opaque and carries a secret and an expiry. mopm has one token scope that matters (`publish`, for
exact `(name, version)` pairs) and no phases, so the stage, commit and read phases are mapped as follows.
Every token is publish-capable for its exact pairs, for stage, commit and yank alike; the phase is a label
in the envelope, not something the server checks until M10.

**Threat model.** Phase separation here guards the engine against its own bugs (a stage grant used to
commit, a read grant used to write). It does not stop a holder of the token, who can call mopm directly.
Server enforcement is gap M10, a new `stage` scope (create, upload, get, abort) with commit and yank requiring
`publish` and `yank`. Until then the grant source mints the shortest-lived token per phase and never reuses
one across phases, and a Stage envelope is handed only to trusted code. After M10 the grant source asks for
the scope of the phase and the table below is also what the server enforces.

Which envelope authorises which verb, per shape (the shape is a construction parameter, so the publisher
checks the table for its own shape):

| Phase | Identity presented | One-call shape allows | Staged shape allows |
|---|---|---|---|
| Stage | publish-scope token, exact pairs | not used | create, upload, abort, group read |
| Commit | publish-scope token (a fresh exchange) | create, upload, abort of own group, commit, poll, yank | commit, poll, yank, group read |
| Read | `Grant::none()` always | package reads only | package reads only |

A test checks that a one-call commit under a Commit envelope sends create, and that the same Commit envelope
in the staged shape refuses create.

The grant secret that the mopm grant source produces and the publisher consumes is a length-framed envelope
in the same crate (module `grants`), so the two cannot drift:

```
version | phase | exceptional | registry location | actor | length-framed list of the five coordinate fields | token
```

The actor (the account or configured identity, an input of the group id) is carried for the staged shape's
group read. Where a client learns it is part of what M1 must document (question 6 in section 16). The
envelope is private to the crate; the secret is never logged and the type stays `!Debug`. It is
unauthenticated: the publisher trusts the grant source that built it.

Rules, each checked client-side first and refused as `GrantRefused(...)` with nothing sent:

- **Presented grant, every verb.** `observe`, `fetch`, `newest` and `list` also check a grant that is
  presented, so a foreign-repository or expired grant on a read is refused and sends nothing (kit scenarios
  18 and 20 run on reads too). Repository and expiry are checked for every verb. Coverage is checked only
  where the verb names coordinates: for `observe` it is `k` plus every coordinate of the declared set (the
  group read reveals the state of the set); `newest` and `list` name none and need an empty coverage list.
- **Read is `Grant::none()`.** No exchange is ever made for a read. A Read envelope that carries a token is
  malformed and refused. Validations, which may run arbitrary code, receive a Read grant only; a test checks
  that no token reaches a validation.
- **Repository binding.** The envelope's registry location is compared byte for byte with the configured
  registry. Mismatch: `Denied`.
- **Phase.** Per the table. A Read envelope or `Grant::none()` cannot write. Wrong phase: `Denied`. This is
  discipline, not server enforcement, until M10.
- **Coverage per coordinate.** Every coordinate of the call must appear, by all five fields, in the
  envelope's list. One missing coordinate, not just the first, is `Denied`. A yank also needs every held
  sibling of its version covered (section 7). The server independently binds the token to exact
  `(name, version)` pairs, at most 100 distinct package names (`grant_limit`).
- **Expiry.** A grant whose `expires_ms` is at or before the injected clock's now is `Expired`, checked
  before every authenticated request and not only at call entry. A 401 `token_expired` from mopm is
  `Expired` before commit is sent and `Unknown` after. An unknown or revoked token is `Denied`.
- **Identity forms.** A write capability needs the federated push identity and a CI identity of a form mopm
  allows for the stage in use (section 3: the configured form on beta, `acts_for` on production). That is a
  setup fact the manual run checks.

### 9.1 Staged state integrity

The group id is a function of the actor and the set, and (until M14) a create of a different set that shares
a member supersedes the actor's own open group. With an account-acting form, every job of that account is
one actor, so a pull-request run, a re-run, or an exceptional run can supersede or reopen the release run's
staged group. `acts_via` also lets any token of the account see every group of the account, narrowed only by
package. Therefore: stage runs only from the protected release identity; exceptional and normal runs of a
machine do not share members; the envelope carries `exceptional` so a log shows it. A superseded or expired
staged group is an expected failure of the staged shape. A `Duplicate` is answered only when the version
record holds the coordinate; otherwise the commit answer is `Transient` (one-call) or, on evidence that
nothing landed, `Permanent` (staged), by section 7's table, so the engine observes and stages again.

### 9.2 The mopm grant source

Section 9 says "the grant source" produces the envelope. For mopm it is a second component, `MopmGrantSource`
(PR 4), in the same crate so the codec has one owner. It checks the grammar and the set of the coordinates it
is asked to cover before it asks for an identity token, because on the beta configured form the exchange
creates missing packages. It exchanges one identity token per phase, because the identity token's `jti` is
single use, and never retries an exchange with the same identity token: a lost exchange response burns it.
`GrantFault::Used` (the exchange says the identity token was already used) and `GrantFault::Unreachable`
(the exchange could not be reached) are mapped in section 10. It applies the `repository_visibility` guard
of section 8. The real exchange (pairs, 10 minutes, single-use `jti`) is exercised only by the manual run
(PR 7); rows 18, 19 and 20 in section 12 prove the envelope logic and nothing about the real exchange.

### 9.3 Lease and token time

A one-call `commit` is one seam call that creates, uploads, commits and then polls for validation, which can
take minutes. The seam has no `Pause` or `Heartbeat` inside a call, and the engine renews its leases
immediately before a `stage`, `commit` or `abort` call and before each grant exchange, never during one.
Two clocks therefore bound the call, and the call must not outlive either:

- the **token**: 10 minutes from the exchange (`MOPM_TOKEN_TTL`, a constant of the crate with a transcript
  test of the exchange response), and the grant's own `expires_ms`;
- the **lease**: the engine's lease guard TTL (600 seconds by default) less the ledger's clock-skew allowance
  (5 seconds). A holder whose lease lapses mid-call is stale: its next fenced ledger write fails, and
  another holder may already have taken over.

The call's budget is the sum of what it may spend: `upload_budget + commit_request_timeout + poll_deadline +
cleanup_timeout`, from `Config`, where `upload_budget` includes the create request and every `PUT` retry,
and `poll_deadline` is the in-call wait for visibility. **Construction-time check:** `MopmPublisher::new` takes `Config.lease_ttl`
(required, no default, set by the caller from the TTL the engine's lease guard uses, because a default here
could silently differ from the engine's) and fails with a configuration error, before anything is sent, unless

```
budget + margin <= min(MOPM_TOKEN_TTL, lease_ttl - lease_skew)
```

where `margin` covers the exchange-to-call gap and one connect timeout. A test builds a configuration one
second over the bound and expects the error, and a configuration at the bound and expects success. Proposed
defaults: upload 180 s, commit request 30 s, poll 240 s, cleanup 15 s, margin 60 s: 525 s against a bound of
595 s.

Per call, with the injected clock: at entry the remaining life of the grant must cover the budget, else
`GrantRefused(Expired)` with nothing sent; the call computes one deadline from the entry time and checks it
before every request. Out of budget before `POST /commit` is written: stop, abort nothing that landed,
`Transient` (the group stays open and a rerun resumes it). Out of budget after it is written, or a 401 on a
poll: `Unknown`. The engine's observe and read-back settle it. `poll_deadline` may be set to zero, which
returns `Unknown` as soon as the commit is accepted, but then only the engine's read-back waits: it polls
`Publish.polls` times (default 3, one tick apart), so a group still validating after that ends
`INDETERMINATE`. The default stays above zero (240 s) and the design asks nothing of the engine here.

**Stale-holder outcome.** If the lease lapses despite the bound (a stalled host, a clock step), the stale
call's commit may still land. That is safe, not harmless-by-luck: the group id is a function of actor and set,
so the successor's identical create resumes the same group (`skip_identical`, then poll), it never makes a
second one; the successor's write-ahead intent is already in the ledger and its observe sees `Present` or
`Absent`; and the stale engine's next ledger write is refused by the fence and it stops. The publisher
returns `Unknown` in that window, never `Permanent`.

**Re-exchange (M2).** The maintainers agreed that a client may, on a 401 `token_expired`, request a fresh
identity token and exchange again (the old `jti` is single use; a fresh token works). That is a grant-source
act, and a publisher receives a `Grant` by value with no way to ask for another. v1 therefore does not rely
on it and the budget check above is the guard. Using it inside a call would need a seam addition (a renewer
handed to the publisher at construction) and is a decision for the seam owners (question 3 in section 16).

## 10. Transport and errors

HTTP client: `ureq` (blocking, small dependency tree, matches the synchronous `Publisher` trait) with
rustls. Constraints from `deny.toml`: no `openssl-sys`, no `native-tls`, permissive licences only, no
wildcards. The crypto provider is `ring` (Apache-2.0 AND ISC) and `webpki-roots` is not used because its
licence is not on the allow list. Both are checked by the licence and ban step of `cargo xtask check` in
PR 1; if `ring` is refused the fallback is to ask for an allow-list change, not to add a C TLS stack.
Archive handling (until M8) uses the pure Rust `zip` (default features off, only the codecs a `.conda`
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
interval, `lease_ttl`, `upload_budget`, `commit_request_timeout`, `poll_deadline`, `cleanup_timeout`, `margin`,
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
| 404 `package_not_found` or 403 `not_owner` on create or yank (the package is not created or not owned by the identity); `limit_exceeded`; 413 `payload_too_large`; 400 `invalid_request` for the daily allowance | `Refused(Reason::Channel)` with a message that names the setup step; nothing landed |
| 404 `package_not_found` or `version_not_found` on a read | `Absent` (observe), `None` (fetch), empty (newest) |
| any other 404, a 404 with an unparsable body, 400 `invalid_name`, `invalid_version`, `upload_invalid` on create | read: `Transient`; create: `Permanent` (the client grammar drifted from mopm; fix the check) |
| 409 `version_exists` on create | `Duplicate` (section 5.1's rule for a sealed version) |
| 409 `group_conflict` (create only) | `Unknown` until M13; then section 6's table |
| 409 `group_commit_refused` | by section 7's table, after the group status read |
| group `rejected` during commit | `Permanent` (nothing landed) |
| group `failed` during commit | `Transient` (numbers burned) |
| 409 `invalid_transition` on yank; 404 `version_not_found` on yank; a coordinate not held (yank); a well-formed body that violates the response schema; an invalid archive or archives that disagree on the manifest (until M8) | `Permanent` |
| 503 `unavailable` on create | `Transient` |
| any transport failure, 5xx, or unreadable answer on a mutating call after commit or yank was sent | `Unknown` |
| failure before commit is sent that changed nothing visible | `Transient` |
| any failure of a read, an unparsable or truncated body included | `Transient` |

`Permanent` and `Refused` are never used after a request that may have landed, except where a status read
proves that nothing did (section 7); the tests prove this with an emulator that applies the commit and then
drops the response, and one that fails the group.

## 14. Decisions

1. **Ship the one-call shape first; build the staged shape behind M1, M13 and M14.** Why: it works against
   mopm as deployed, and the alternative (observing staged state by re-sending create) violates `observe`
   being a read. Alternatives: refuse mopm until M1 (delays the first real channel for a dependency we do
   not own); keep a local record of group ids (a second source of truth, lost with the machine, and it fails
   the lost-answer scenarios).
2. **One crate, `ucdk-release-mopm`, containing the publisher, the grant envelope codec and the mopm grant
   source (PR 4).** Why: the envelope is a private contract between its halves. Alternative: a separate
   grant-source crate (two crates must agree on an undocumented format).
3. **`CommitUnit::Set`, `conditional_put() == true`.** Why: group commit is all-or-none and create is
   compare-and-set. Alternative `Coordinate` would break atomicity across platforms of one version.
4. **`observe` reads the package record, never `list` or search; the read-back fetches and the engine
   hashes.** Why: seam rule; the record is the authoritative per-version read, and a registry's own digest
   proves only what the registry says.
5. **A version sealed with another file set is `Refused(Channel)` on write and `Absent` on observe, never
   `Duplicate`, except a burned (tombstoned) record, which is `Duplicate` and reads `Burned`; a
   multi-platform release must publish all platforms in one set with one manifest (section 5.1).** Why: avoids
   an observe-`Absent`, commit-`Duplicate` loop; mopm will not change. Alternative: widen `Present` to the
   whole version (lies about a coordinate not held).
6. **`Committed` is returned at `visible`.** Why: the seam's "readable" then holds on every mopm read path.
   Alternative: return at `committed` (shorter call, but repodata can lag the answer).
7. **`yank_unit() == Version`; the engine names and covers every sibling, and the publisher refuses
   otherwise.** Why: mopm has no finer unit and a yank must not exceed its grant. Alternative: a silent
   widening (exceeds coverage and leaves the ledger disagreeing with the registry).
8. **Phase, repository, coverage and expiry are enforced client-side, on every verb.** Why: "nothing is
   sent" is a client-side promise anyway, and the server has one scope until M10. These rows prove envelope
   logic only.
9. **`ureq` with rustls, `ring`, and `Roots::Bundle(path)` only; blocking; an injectable `Transport`.** Why:
   matches the synchronous trait, the deny list and the no-environment rule. Alternatives: `reqwest` (drags
   an async runtime); `hyper` directly (more code to cover at 99%).
10. **Emulator in process, pure, as the rig; a manual non-production run for fidelity; no prod.** Why: CI
    is Rust only and never reaches a live registry. Alternative: drive mopm's own server from CI (adds a
    TypeScript runtime to CI).
11. **Manifest hash computed client-side from the archive, until M8; then the code is deleted.** Why: not
    blocked on M8, and no second path survives it.
12. **`grant_limit()` is 100 coordinates; a set with two versions of one name is refused.** Why: a bound in
    coordinates is never over mopm's distinct-name limit. Alternative: count distinct names (a trait and
    two kits change).
13. **A PRIVATE or UNSPECIFIED repository is refused at construction, an anonymous push identity cannot
    write, and the grant source guards on `repository_visibility`.** Why: mopm cannot honour private reads
    and an anonymous write has no token.
14. **No environment variables.** The registry, timeouts, caps and trust bundle are configuration; the
    credential is the grant.
15. **No `group_conflict` message is parsed; every one is `Unknown` until M13's reason enum.** Why: a
    wording change must not change a class, and a wrong `Duplicate` or `InFlight` loops the engine.
16. **A 404 is `Absent` only with a parsed `package_not_found` or `version_not_found` body.** Why: a
    mis-pointed registry must not look empty.
17. **A tombstoned record is never `Absent`: `Burned` without a digest, `Yanked` with one.** Why: a burned
    number can never be committed again, and a terminal state must be a refusal, not a retry.
18. **A failed group answers `Transient`, never `Permanent`.** Why: the burn is an effect; the engine's
    observe reads `Burned` and refuses. `Permanent` is allowed only where a status read proves nothing landed.
19. **A call is bounded below the lease and the token, checked at construction (section 9.3).** Why: the
    seam has no heartbeat inside a call and the engine renews only before it.
20. **`on_overlap: refuse` is relied on only when the create response shows it was honoured (section 6).**
    Why: an old server ignores the field and supersedes silently.

## 15. PR plan

Each PR is small, passes `cargo xtask check` and the 99% line and branch coverage bar for the crates it
touches, and ships tests that failed first for any fix.

1. **Skeleton and mapping types.** Crate manifest (not `pure`), licence headers, `Config` (no default
   registry, `Roots::Bundle` only, `lease_ttl` and the construction-time budget check), `Transport` trait
   and types, `Secret` types, coordinate-to-member mapping with mopm's exact grammars and the platform
   table, set checks, error mapping table, the `ureq` dependency checked against `deny.toml`. Tests:
   mapping and injectivity (coordinates differing only where a `/` falls), the platform table both ways,
   grammar refusal for observe too, an invalid-grammar version never produces a request, every row of the
   error table, the budget check one second over and at the bound, a bogus certificate variable changing
   nothing, the version length, empty file, 20 GB and per-member manifest agreement refusals, the archive's
   subdirectory against the table, and the `Retry-After` ceiling.
2. **One-call `commit`, `fetch`, `yank`.** Create, upload, commit, poll; the create-answer classification
   (including 200 `skip_identical`, every `group_conflict` as `Unknown`, the sealed-version rule and the
   tombstoned `version_exists` as `Duplicate`);
   manifest hash extraction module; the commit status table; `Unknown` after send. Tests on a hand-built
   fake `Transport`, including apply-then-drop, the `validating` window with `observe` giving `Absent`, a
   failed group answering `Transient`, and the yank refusals (not held, uncovered sibling).
3. **`observe`, `newest`, `list`.** Package read, 404 kinds, tombstone mapping (`Burned`, `Yanked`), the
   sparse-index read with its fallback, ambiguity rule, caps. Tests for lag, failed reads never `Absent`
   (a bare 404 first), a failed read of the third of five versions, no digest, a burned member skipped.
4. **Grants.** Envelope codec, repository, phase (per shape), coverage, expiry checks on every verb with an
   empty request log on refusal; the 401 mappings; `MopmGrantSource` (one identity token per phase, `Used`
   and `Unreachable`, the grammar check before the exchange, the visibility guard).
5. **Emulator and rig; kit.** `ucdk-release-mopm-emulator` (section 11), `MopmRig` with the hooks, and
   `run_publisher_kit` for the one-call shape, plus the TCP-stub adapter tests. Lands after kit parts 6 and
   7 (4g, 4h).
6. **Staged shape** (after M1, M13, M14): the group read, `stage`, `abort`, staged `commit`, `observe`
   with a declared set, `on_overlap: refuse` and its honoured check (a server that ignores the field is
   refused); extends the emulator and runs the staged shape through the kit.
7. **Manual non-production conformance job** script and its documentation: protected environment, manual
   dispatch, the non-production marker check, and the checklist of section 11. Scenarios are marked proven
   against real mopm only after this run.

Outside this crate and in this order of need: the engine and machine change of section 5.1 (before any
multi-platform release); the maintainers' M13 and M14, then M1, M6 and M8, then M10, then M3 (section 13);
the yank verb's sibling rule in the engine.

## 16. Seam owners' answers, what changed, and open questions

**Answered (taken).** The seam owners decided: a burned coordinate is `Presence::Burned` with
`PUBLISH_BURNED` (not `Yanked` with an empty digest); `Publisher::yank_unit()` exists and a version-unit yank
needs every sibling named and covered; the tick is `PAUSE_TICK_MS` (one second) and `Backoff::after_seconds`
converts; `observe` reports the digest the registry itself verified at upload, and the read-back fetches and
hashes; `CONDA` has the file-bytes identity; a failed observe is `REFUSED` before an effect for `Refused`,
`Permanent` and a denied grant, and `INDETERMINATE` after one; a rig declares its shapes, and a skipped
scenario is reported and never counted as a pass; the rig supplies `shapes()`, `coordinate()`, `payload()`,
`siblings()`, `unrepresentable()`, `seals_versions()` and `can_burn()`; the kit computes the expected digest
from the bytes it gave; `Roots::Bundle(path)` is the only v1 trust source; the seam counts coordinates and
100 is safe.

**What revision 3 changed.** A burned coordinate reached by commit is `Duplicate` (5.1); `on_overlap` is
trusted only on evidence (6); one manifest per member, and the client checks for it (5.1, 5); the `Retry-After`
ceiling (10); the engine's own poll bound stated as it is (9.3, 6); `darwin-aarch64` is the macOS token; a
yank of an unheld coordinate and an unparsable read body are `Permanent` and `Transient` as the table says;
abort of a validating group is `InFlight`; group-allow-list, cache, index and sealing facts corrected to the
source (3); budget terms named once (9.3); the scenario counts are split (12).

**What revision 2 changed.** Multi-platform releases are stated as impossible under the current stage model
(5.1) with the engine's part named; the platform mapping is specified; error classes follow the burn (failed
is `Transient`, a burned coordinate is `Burned`, every `group_conflict` is `Unknown` until M13, never
`Duplicate`); the lease and token section with its construction check is new (9.3); yank is per version with
the publisher's own refusals; the rig supplies shapes and coordinates and `types()` is `CONDA`; the read-back
fetches and the engine hashes; the configured-publisher exchange creates unclaimed packages and
`name_reserved` is a claim error; roots are one bundle; section 13 is the maintainers' verdicts with the
agreed order.

**Open questions**, each with a recommendation:

1. **Multi-platform mechanism (engine owners).** (1) a declared platform set, (2) one joined Publish set, or
   (3) refuse at admission a split publish for a sealing channel. Recommend (3) now and (1) with (2) next.
2. **The stamp has no home on mopm.** The kit's base `Held` carries a stamp (build number and source commit)
   and an exceptional mark, and the list, newest and stamp scenarios compare the whole `Held`. The
   maintainers took a typed `source_commit` (M3) only. Recommend asking for `build_number` and `exceptional`
   as typed fields too, and moving M3 ahead of M10; the alternative is a seam declaration that a channel
   stores no stamp, with the engine refusing a monotonic rule on it.
3. **Re-exchange inside a call** needs a renewer handed to the publisher (a seam addition). Recommend not
   adding it in v1; the budget check suffices.
4. **Burned handling after a `Transient` stage, and a poll bound for validation (engine owners).**
   `release.md` states the observe that reads `Burned` after a `Duplicate` and in the commit steps, not after
   a `Transient` in the stage phase, which the staged shape relies on for a failed group. And the read-back
   bound (3 polls) is shorter than a minutes-long validation if `poll_deadline` is zero. Recommend stating the
   first and leaving the second as the construction check in 9.3.
5. **A staged commit on a dead group whose version is not held answers `Permanent`**, on the evidence of the
   status read (the kit's lost-staged-state scenario requires an error that promises no effect). The review
   asked for `Transient`; the difference is whether the engine retries the same commit or restages.
   Recommend `Permanent`, and say so in the runbook.
6. **The actor string** is an input of the group id. Ask M1's documentation to say where a client reads it
   (the exchange response is the natural place).
