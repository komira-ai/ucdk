# The mopm release Publisher

Status: design. Nothing in this document is implemented yet. Where it says "the publisher does X" it
describes what the crate will do when the PRs in section 15 land.

## 1. Summary

ucdk's first real release channel is a mopm registry. This document designs `ucdk-release-mopm`, a Rust
crate that implements `ucdk_release_seams::Publisher` against mopm's HTTP API (the group publish path and
the public read path) and is held to the publisher kit (`run_publisher_kit::<R: PublisherRig>`, 39
scenarios, nine contract points).

Two facts shape everything:

1. mopm has a server-side staged set: a *group*. It is open and invisible, it is create-if-absent per
   `(name, version)`, and its commit is all-or-none. That is a good fit for `CommitUnit::Set`.
2. mopm cannot be asked what a group holds. `GET /groups/{id}` returns member states, never digests or the
   declared file list, the group id derivation is not a documented interface, and the only call that finds a
   group from its declared set is `POST /groups`, which is a write (it reopens dead groups, charges counters
   and supersedes overlapping groups). `observe` must be a read. So `Staged` cannot be observed today.

The design therefore ships in two shapes of one publisher:

- **One-call shape** (`stages() == false`). `commit` creates the group, uploads, commits and waits for
  visibility in one call. It works against mopm as deployed today. `stage` and `abort` are never called.
- **Staged shape** (`stages() == true`). `stage` creates the group and uploads, `commit` commits it. It
  needs one mopm change (gap M1, a read-only group lookup) and is built behind it.

The shape is a construction parameter, not a runtime guess, and there is one code path for each verb (no
parallel API): the staged shape adds the stage and abort verbs and changes what `commit` sends; everything
else is shared.

mopm is a public registry for Mojo library packages. It has no private visibility, no per-phase scopes, no
place to store a build stamp, and it accepts only valid Mojo library archives of a fixed name grammar.
Section 13 lists the gaps with a recommended mopm change each and says what the publisher does meanwhile
(refuse, or degrade with the degradation named). Section 12 says, scenario by scenario, which are met, which
are met only on the emulator, and which are blocked on a gap.

Three things in this document are stated as limits of the evidence, not as facts about the product:

- Everything in section 3 was read from the mopm sources, not executed.
- The in-process emulator (section 11) copies this document's reading of mopm. A scenario that passes on it
  is evidence about the publisher, not about mopm, until the manual run of PR 7 repeats it against a real
  deployment (section 12 says which rows that applies to).
- The kit and seam owners answer the questions in section 16; where a row depends on an answer it says so.

## 2. Goals and non-goals

Goals

- Implement every `Publisher` method with the semantics in `publisher.rs`, and pass the kit for the shapes
  the publisher declares.
- Never decide an effect on a lagging read. `observe` reads the authoritative per-version record; `list` is
  used for reports only.
- A mutating call whose transport failed answers `Unknown`; a registry that committed and then failed never
  answers `Permanent`.
- Refuse, at construction or preflight, what mopm cannot honour, instead of faking it. Refuse before any I/O
  whatever mopm would answer with a 400, so that no request that can never succeed is ever sent or retried.
- Client-side enforcement of grant repository, phase, coverage and expiry, so a refused call sends nothing.
- All timeouts, retry counts, page caps and the registry location are configuration passed in by the
  caller, never environment variables.

Non-goals

- The OIDC exchange itself. It belongs to the grant source (section 9 gives the envelope both sides share,
  and PR 4 builds the mopm grant source that produces it).
- Private repositories. mopm has none; the publisher refuses a PRIVATE repository.
- Claiming a package name. A mopm package must exist and be owned by the publishing identity before any
  group can name it (section 3); the publisher never claims one.
- Reaching a live registry from CI. CI runs the kit against an in-process emulator (section 11).
- Changing the seam. Where this design needs a seam clarification it is listed as a question (section 16)
  because a seam lands whole.
- Any change to mopm from this repository. mopm changes are recommendations (section 13).

## 3. mopm facts the design relies on

Read from the mopm sources at commit d062f75 of the public mopm repository (`service/api/src` unless a
path says otherwise); not executed. Each bullet names the file and function a reviewer can open.

**Package preconditions**

- `POST /groups` requires every member's package to exist and the caller to own it: `requireOwners` in
  `publish/group-service.ts` calls `requireOwner` in `publish/service.ts`, which answers 404
  `package_not_found` or 403 `not_owner`. An account token must belong to a current owner; a CI token must
  name the package's `ci_owner`. Claiming a package is a user-token route, not a CI-token one
  (`http/publish-routes.ts`). So the first publish of a new name needs a human or an operator to claim it
  first and to set the CI owner. The publisher cannot do this and the kit scenarios that commit a fresh
  coordinate presuppose it (section 11, rig step 1).
- Group publish is off by default on every stage. When it is off, create answers 404 `route_not_found`
  (`http/publish-routes.ts`, the groups switch).
- Create can also answer 403 `name_reserved`, 429, 413 `payload_too_large` (a file above 100,000,000 bytes,
  decimal), 400 `invalid_request` (including more files than the daily file allowance), and
  `limit_exceeded` (per package size quota). A 503 `unavailable` answers when a queue is down.

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
  (`GET /groups/{id}`), abort (`DELETE /groups/{id}`, only an `open` group), commit
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
  preflight. If it is `failed`, answer 409 `group_conflict` (numbers burned). Otherwise open or reopen it,
  preflight, take a slot, supersede, reserve.
- Preflight and reservation: a `(name, version)` already published, yanked or tombstoned refuses the whole
  create with 409 `version_exists` and nothing is reserved. A pending row held by another live group, the
  open-group cap (5 per actor), the supersede cap (a create may supersede at most 3 of the actor's own open
  groups) and a failed group retried are all 409 `group_conflict`. The response body of every error carries
  only `{code, message, hint}`. A number taken by another publish after the preflight aborts our group on
  the server and answers `group_conflict`; the group is not left open, its rows are free, and the next
  identical create is a fresh attempt, charged again against the daily file allowance.
- A create by an actor with a different set that shares a member supersedes the actor's earlier open group
  (`supersede` in `group-service.ts`) and answers 201.
- Group states: `open`, `validating`, `committed`, `visible`, `rejected`, `failed`, `expired`, `aborted`,
  `superseded`. `rejected` frees the numbers. `failed` burns them: the validator applies `group_burn`, which
  turns each pending member into a `tombstoned` record (`state.ts`, `publish/group-validator.ts`).
- Commit (`publish/group-commit.ts`): 202 when `started` is true, and `started` is true both for a fresh
  commit and for an identical repeat while the group is still `validating`; 200 for `committed` and
  `visible`. A commit on an `expired`, `aborted`, `rejected`, `superseded` or `failed` group is 409
  `group_commit_refused`. A commit with an unreserved member is refused ("repeat the create request"),
  and a missing file is 400 `upload_invalid`; both are recoverable by repeating the identical create and
  uploading. DELETE on a group that is not `open` is 409 `group_not_open`; 404 `group_not_found` is also
  what a non-creator receives.

**Reads (public, `cache-control: no-store`; `http/router.ts`)**

- `GET /packages/{n}`, `GET /packages/{n}/versions` (paged, limit 1 to 100, cursor),
  `GET /packages/{n}/{v}`, `GET /packages` (search: normalised lowercase prefix of at most 64 characters,
  sorted, cursor). Pending versions are invisible to every read route. After the group is `committed` the
  package API reports the members `published`; repodata and index follow, and `visible` comes last.
- Version detail has `state` (`published`, `yanked`, `tombstoned`), `yank_reason`, `tombstone`,
  `files[]` with `sha256` and `size` (and `url` unless the version is tombstoned), and `manifest_sha256`.
  A tombstoned version is returned with 200, never 404: a takedown keeps each file's `sha256`; a version
  burned by a failed group may have an empty file list. (The two sources of this fact disagree on which of
  the two a burned member shows; the design handles both and the manual run records which, section 11.)
- `GET /packages/{n}` returns the unpaginated `versions` list, tombstoned versions included. The search
  `latest` is the newest published version only, excluding yanked and tombstoned, and is null when there is
  none.
- 404 comes in kinds: `package_not_found`, `version_not_found` (both genuine absence), `route_not_found`
  (an unknown path, or the index route not configured) and, from a gateway, a body that is not JSON.
- Digests are lowercase 64-hex SHA-256, the same form as `Hash256::as_str`; no conversion is needed.
  Reads are served from a strongly consistent store as deployed (`ConsistentRead`), so the version record
  is uncached and read-after-write exact at the origin; a gateway layer is not known and is covered by the
  manual run.

**Tokens** (`auth/oidc.ts`, `auth/tokens.ts`, `http/publish-routes.ts`)

- The CI exchange mints only `publish`-scope tokens, bound to exact `(name, version)` pairs, 10 minutes,
  and the identity token (`jti`) is single use. The pair list names each package at most once ("names a
  package twice" is refused), so the 1 to 100 limit counts distinct package names.
- There is no `yank` scope for a CI token. Yank by CI works because `authorizeYank` falls back to the
  publish token for the exact `(name, version)`. So every token is publish-capable for its exact pairs, and
  the same token that staged can also commit and yank.
- A configured-publisher identity (`ci_owner`) is permitted on the beta stage, and on production only with
  an `acts_for` account; a CI token that is not configured cannot create a group. The group actor is the
  account when acting for it, or the configured CI identity otherwise (`publish/service.ts`,
  `ci-publishers.ts`); either way two jobs of one actor share the open-group cap and supersede each other.

**Yank**: `PUT /packages/{n}/{v}/yank`, idempotent (a repeat returns the record unchanged and keeps the
first reason), reason 1 to 280 characters, owner only, not gated by read-only mode, needs the index queue
(503 `unavailable` otherwise). A pending or tombstoned version is 409 `invalid_transition`; a missing one
is 404 `version_not_found`. The channel specification states that the file stays downloadable at its exact
URL after a yank, and version detail keeps `url` for a yanked version.

**Limits**: 500 members and 20 files per member per group, 100,000,000 bytes per file, 12 hour group
deadline.

## 4. The mapping

`Publisher` capability getters, per shape:

| Getter | One-call shape | Staged shape | Why |
|---|---|---|---|
| `types()` | `[mopm Mojo library package]` | same | mopm accepts only Mojo library archives with an embedded manifest, not generic conda |
| `stages()` | `false` | `true` | see section 1 |
| `commit_unit()` | `Set` | `Set` | a group commits all or none |
| `conditional_put()` | `true` | `true` | create refuses an existing version (`version_exists`), commit is compare-and-set on group status |
| `grant_limit()` | `100` | `100` | the exchange covers 1 to 100 distinct package names; the seam counts coordinates, so 100 is a conservative bound (Q4) |

Method mapping. "Public" means no token. "Token" means the grant's secret as bearer, attached only to the
calls that say so and never to a read or a file URL.

| Method | mopm call | Grant used | Error mapping (summary; full table in section 10) | Idempotency and atomicity argument |
|---|---|---|---|---|
| `observe(g, k, set)` | One-call shape: `GET /packages/{n}/{v}` (public). Staged shape: first the read-only group lookup keyed by `set` (M1, token), then the same package read. | none needed. A grant that is presented is checked for repository and expiry on every verb, and for coverage where the verb names coordinates (section 9). | A 404 whose parsed body code is `package_not_found` or `version_not_found` gives `Absent`. Any other 404, an unparsable 404, and every read failure is `Transient` (`Refused(Channel)` for `route_not_found`), never `Absent`. `state: tombstoned` is `Yanked` or `Permanent` (section 8). | A read. Exactness rests on the version record being uncached and strongly consistent (section 3, proven against a deployment by the manual run). Never calls `list`, search or create. |
| `stage(g, items)` (staged shape) | `POST /groups` (repeated with the identical body while `reserving`), then `PUT` each file the response says is not held, with the returned headers and no token | stage grant | 409 `version_exists` gives `Duplicate`; 409 `group_conflict` is classified by section 6; 200 on an own group with every member `skip_identical` gives `Duplicate` | Group id is a function of actor and declared set, so a repeat of the identical set resumes the same group. A different set of the same actor that shares a member supersedes instead (gap M14). |
| `commit(g, items)` one-call | create, upload, `POST /groups/{id}/commit`, poll `GET /groups/{id}` until `visible`; the create answer is classified by section 7 (own group already `validating`, `committed` or `visible` skips upload and commit) | commit grant (covers create, upload, abort of own group, commit, poll, yank) | see sections 7 and 10 | one group is one atomic commit; `version_exists` is the create-if-absent |
| `commit(g, items)` staged | `POST /groups/{id}/commit` with the manifest hashes, poll to `visible`. The answer is decided by the group status in the body, not the HTTP status. | commit grant | body status already `committed` or `visible` gives `Duplicate`; `validating` is polled and gives `Committed` at `visible` | an identical commit body is idempotent server-side; a different body is 409 `group_commit_refused` |
| `abort(g, ks)` (staged shape) | `DELETE /groups/{id}`; on 409 or 404, `GET /groups/{id}` | stage grant | section 7 table | DELETE is idempotent only for `aborted`; the follow-up read makes the rest idempotent |
| `yank(g, k, reason)` | `PUT /packages/{n}/{v}/yank` with `{yanked: true, reason}`; reason of 1 to 280 characters checked first | commit grant covering `k` | 409 `invalid_transition` and 404 `version_not_found` give `Permanent` | server-side no-op on repeat. Coarser than the seam: yanks the whole version (section 7). |
| `fetch(g, k)` | `GET /packages/{n}/{v}`, then `GET` the file `url` from that record | none | 404 version, no such file, a tombstoned version (no url) give `None`; digest mismatch gives `Permanent` | Never builds a URL itself, so staged bytes are never returned. The URL's origin is checked against configuration (section 10). |
| `newest(g, ch, name)` | `GET /packages/{n}` (versions, newest first, yanked and takedown-tombstoned included), then `GET /packages/{n}/{v}` per version | none | see rules below | A read of committed state; compares nothing. |
| `list(g, scope)` | `GET /packages?sort=name` paged, stop once names pass the prefix, then the latest version's record per package | none | read failures are `Transient` | Reports only; omitting older versions, yanked-only and tombstoned-only packages (search `latest` is null for them) is the permitted lag. |

`newest` rules. No package (404 `package_not_found`) and a claimed package with no published version give an
empty list, not an error. A version burned by a failed group (no digest) is skipped, and a takedown
tombstone is listed as `Yanked`. If any one of the per-version reads fails, the whole call fails
(`Transient`), and a partial list is never returned. A versions list over the configured cap is `Permanent`,
never silently truncated. (A paged `/packages/{n}/versions` source may replace the unpaginated list if M7
lands.)

## 5. Coordinates and keys

A ucdk `Coordinate` is `{channel, type, name, version, platform}`. mopm's identity is `(name, version)`, with
files `mojo-<name>-<version>-<build>.conda` under a subdirectory. The client check uses mopm's exact
grammars (section 3), and everything outside them is `Refused(Reason::Channel)` before any I/O, for
`observe`, `fetch`, `newest`, `stage`, `commit`, `yank` and `abort` alike. A name that mopm would answer with
400 `invalid_name` can never be read as `Transient` and retried:

- `channel` must equal the channel the publisher was constructed for, else `Refused(Reason::Channel)`.
- `type` must be the one type in `types()`, else `Refused(Reason::Channel)`.
- `name` must match `^[a-z][a-z0-9_]{1,63}$`. So `Demo` and `my-pkg` are refused locally.
- `version` must match exactly three dot-separated numbers without leading zeros. So `1.0.0-rc1`, `1.0` and
  `01.2.3` are refused locally.
- `platform` is one of `noarch`, `linux-64`, `linux-aarch64`, `osx-arm64` (the `SUBDIRS_V1` list, which the
  publisher takes from one table in the crate and a test compares with section 3); any other value is
  `Refused(Reason::Channel)`.
- The file is the `Blob` whose `name` is `mojo-<name>-<version>-<build>.conda` with the coordinate's name and
  version, a build matching `^mojo[0-9]+(?:\.[0-9]+)*_0$`, and at most 255 characters. A mismatch is
  `Refused(Reason::Channel)` before anything is sent. A blob that is not a valid package archive (it has no
  embedded manifest) is `Permanent` before any I/O (section 13, M8).

Consequences, each enforced in code:

- **One file per coordinate.** A registry version may hold several builds in one subdirectory. `observe` of
  a coordinate whose `(version, platform)` holds more than one file answers `Refused(Reason::Channel)`
  ("ambiguous"), never an arbitrary one.
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

**A version is sealed by its first group.** A later set that names an extra platform of an already published
version cannot be created. The publisher distinguishes the cases after a 409 `version_exists` by reading the
version record: every requested coordinate present with the same digest gives `Duplicate`; a requested
coordinate present with another digest gives `Duplicate` (old bytes stand); any requested coordinate missing
gives `Refused(Reason::Channel)` ("the version is sealed with another file set"). `observe` of the missing
coordinate answers `Absent`, so the engine cannot loop observe-Absent, commit-Duplicate.

This is a deliberate mismatch with kit contract point 3 and scenarios 32, 36 and 8, which expect `Duplicate`
whenever any member of the set is held. It is a gap, not a closed question (row GAP-S in section 13, and
Q9): if the kit builds a set from two platforms of one `(name, version)`, as the fake allows, the design
fails those scenarios by choice. The answer `Refused` still promises that nothing happened. Q9 asks the kit
owner to vary the name, or to agree that this case is `Duplicate` together with an `observe` that reports
the missing sibling.

## 6. Staging is a mopm group

Applies to the staged shape.

- `stage` computes the members (one per `(name, version)`, files sorted), sends `POST /groups` and repeats
  the identical request while the answer is `reserving` (bounded by configuration).
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
- 409 answers are classified by the table below. `group_conflict` is one code for several unrelated
  conditions, so the code alone is never enough:

| Condition (message in mopm) | Staged shape | One-call shape |
|---|---|---|
| `version_exists` | `Duplicate`, with the sealed-version rule of section 5 | same |
| a member "is held by group X, which is <status>", or "belongs to group X, which has no header" | `Duplicate` (staged by another) | `InFlight` |
| "you already have N open groups" (the open-group cap) | `Refused(Reason::Channel)` ("abort an open group") | same |
| "shares versions with N of your open groups" (the supersede cap) | `Refused(Reason::Channel)` | same |
| "group X failed ... its version numbers are burned" | `Permanent` ("numbers burned; publish a new version") | same |
| "a number was taken" (our group was aborted by the server) | `Duplicate` if the version record now shows the coordinate held, else `Transient` once | same, `InFlight` if held by a pending group |
| "your group X is <status> and shares a version" | `Refused(Reason::Channel)` | same |
| any other `group_conflict` message | `Refused(Reason::Channel)` ("unclassified conflict") and a transcript is recorded | same |

  The error body has no structured field, so the classification parses the message prefix. That is
  fragile on purpose and fails closed: an unknown message never becomes `Duplicate` or `InFlight`. Each
  row is pinned by a recorded-transcript test and an emulator case, and the ambiguity is gap M13 (distinct
  codes). A caller over its open-group cap is never told `Duplicate`, so the engine cannot loop
  (commit gives `InFlight` or `Duplicate`, observe gives `Absent`, repeat).
- **Same-actor supersede (gap M14).** Create of a set different from an earlier open set of the same actor,
  sharing a member, answers 201 and discards the earlier group's staged bytes. The publisher cannot see that
  without M1. So `stage` over one's own staged coordinate with different bytes answers `Staged`, not
  `Duplicate`, and the first staged bytes are gone; only a stage of the identical set is `Duplicate`. The
  kit's `a_stage_over_ones_own_staged_coordinate_is_duplicate` and
  `duplicate_over_other_bytes_leaves_the_old_bytes` vary the bytes, so rows 34 and 32 are GAP-M14 for the
  staged shape until mopm offers a no-supersede flag on create, or the staged publisher runs the M1 lookup
  before create and answers `Duplicate` itself.
- Create is sent once per attempt and never blindly retried after an `Unknown`; the engine observes first.
  Two exceptions, each bounded: the identical create while `reserving`, and a re-create to refresh upload
  URLs (section 10).
- `observe` must not call create. Create reopens dead groups, charges daily file counters and can
  supersede another open group sharing a version. A "read" with those effects would break the seam. Hence M1.
- Staged state expires (12 hour deadline, lazily). A staged coordinate whose group expired reads `Absent`;
  the engine's observe-then-stage then creates a fresh attempt. Abandoned open groups use one of the 5
  open-group slots of the actor until then, which is an operational note for the staged shape.
- A conflict mid-reservation reserves nothing that stays: a preflight conflict writes nothing and releases
  the slot; a number taken after the preflight aborts the group on the server. There is nothing to give back
  by an abort-through-lookup in that case (the old gap M4 is withdrawn, section 13).

## 7. Commit, lost answers, abort and yank

### Commit

Common steps, in order:

1. Validate the call with no I/O: grammar and set checks (section 5), the package archive and its
   `manifest_sha256` (section 13, M8), grant checks (section 9), member grouping.
2. One-call shape: create, then classify its answer.
   - 201, or 200 on an open group: upload every file the response lists, then go to step 3.
   - 200 with every member `skip_identical` on an own group whose body status is `validating`: this is our
     own commit still in flight (the group id is a function of actor and set). Skip upload and commit and go
     to step 4, which answers `Committed` at `visible`.
   - 200 with every member `skip_identical` on an own group already `committed` or `visible`: nothing
     changed by this call. `Duplicate`; the engine's following `observe` says whether the held digest is
     ours (the seam leaves the digest of a `Duplicate` to `observe`).
   - 409: section 6 table. Failure here changes nothing visible; the publisher best-effort aborts its own
     group and answers `Transient` for transport failures.
3. `POST /groups/{id}/commit`, sent once. From the moment its first byte is written, every failure
   (connect, timeout, 5xx, lost response, unreadable body) answers `Unknown`, per the seam. A 409
   `group_commit_refused` on a dead group is followed by a read of the version record: held gives
   `Duplicate`, not held gives `Permanent`. A refusal that says a member is unreserved, or a 400
   `upload_invalid` for a missing file, is recoverable by repeating the identical create and uploading, and
   gives `Transient`.
4. Poll `GET /groups/{id}` (token) every configured interval until `visible` or the configured deadline.
   `rejected` gives `Permanent` (numbers freed, nothing landed). `failed` gives `Permanent` with a message
   that the numbers are burned. Deadline, 401 on a poll (token expired, M2) or a read failure gives
   `Unknown`.
5. `Committed` is returned only at `visible`, so every read path of every member shows the set.

Group status table for a resumed one-call commit and for a staged commit (the body status decides, not 200
versus 202, because 202 is also the answer to a repeat while `validating`):

| Group status at entry | Answer |
|---|---|
| `open` | upload what is missing, commit, poll |
| `validating` | poll; `Committed` at `visible`, `Permanent` on `rejected` or `failed` |
| `committed`, `visible` | `Duplicate` |
| `failed` | `Permanent` (numbers burned) |
| `rejected`, `expired`, `aborted`, `superseded` | create reopens the group as a new attempt (one-call); a staged `commit` gives `Permanent` or, if the version is held, `Duplicate` |

All-or-none: the group is one unit. `rejected` before the copy phase publishes nothing and leaves the
coordinates `Absent`. A `failed` group, after the copy phase, burns the numbers: channel files may sit at
exact URLs, and every member is a tombstoned record. That is **not** `Absent`: `observe` of a burned
coordinate answers `Permanent` ("number burned by a failed group"), or `Yanked` if the record kept a digest,
as section 8 says. A later `commit` of that coordinate answers `Duplicate` (create preflight, `version_exists`),
never a retry. Visibility across subdirectories and index files is per read path, not simultaneous;
`Committed` waits for `visible`, and `observe` reads the package record, which flips at `committed`. A
reader of repodata can lag `observe`. That is why the smoke action polls (Q6).

Atomic against a coordinate appearing between observe and commit: create's `version_exists` is the
compare-and-set. The answer is `Duplicate` and nothing is created. In the staged shape the reservation makes
the race impossible after `stage`; the race exists only during `stage`.

Lost answers: `commit` answering `Unknown` or `Transient` is resolved by `observe`. The group can then be in
three places. `visible`: `observe` shows `Present` with the declared digest, and a retried commit answers
`Duplicate`. `committed` or `validating`: after `POST /commit` is accepted the group is `validating` and
package reads still show `Absent` until `commitGroup` sets `committed_at`; a retry's create answers 200
`skip_identical` and the publisher goes to polling (step 2), so the retry answers `Committed`, never
`Duplicate`, and cannot loop. Never started: `observe` shows `Absent`, and the retry re-sends the identical
create (the same group, resumed) and commit. In every case the following `observe` tells own bytes from
foreign ones; the publisher does not decide that. A test holds the `validating` window with `observe` giving
`Absent`, and a second test applies, drops the answer, lets the group reach `visible`, retries and expects
`Duplicate` and then `Present` with our digest.

### Abort (staged shape)

`abort(g, ks)` finds the group by lookup (M1) and sends `DELETE`:

| mopm answer | Publisher answer |
|---|---|
| 200 or already `aborted` | `Ok` |
| 404 `group_not_found` | `Ok` only if the lookup (or the recorded group id) agrees the group is gone; otherwise `Unknown` (a non-creator and an uncovered token also get 404) |
| 409 `group_not_open`, then `GET`: `expired`, `rejected`, `superseded`, `aborted` | `Ok` |
| `committed`, `visible` | `Ok`, touches nothing (only yank can undo) |
| `validating` | `Ok` without effect; a later commit reports it |
| `failed` | `Ok` (nothing to give back; numbers are burned) |
| 401, 403 | `GrantRefused` |

In the one-call shape the engine never calls `abort`; the publisher still gives back its own half-built
group inside a failed `commit` (step 2), and a failed cleanup is expected and is resolved by resume.

### Yank

`yank(g, k, reason)` sends the single-version yank after checking that the reason is 1 to 280 characters
(else `Refused(Reason::Channel)`, nothing sent). It is coarser than the seam: mopm yanks a `(name, version)`,
so every platform coordinate of that version reads `Yanked` afterwards. This is truthful (`observe` of a
sibling says `Yanked`), documented here, and a question for the engine owner (Q3). It is also a security
fact: the effect exceeds a grant that covers one coordinate, so the publisher refuses a yank when the
version record holds other coordinates than `k`, unless the grant covers all of them (an unambiguous full
version yank). Bytes stay fetchable: the channel specification says so, and the version record keeps the
`url`; the manual run proves it for the real channel host.

A yanked coordinate can never be committed again: the version row persists, so create answers
`version_exists`, which maps to `Duplicate` (the seam's fake does the same, and the rig accepts any answer
other than `Committed`).

Yank by group (`PUT /groups/{id}/yank`) is not used: the group id of a published version is not readable
(gap M6).

## 8. observe on a private registry, declared set and empty set

mopm has no private visibility, so the publisher declares itself public-only and refuses to be constructed
for a `PRIVATE` repository, for an `UNSPECIFIED` visibility, and, for any write capability, for an anonymous
push identity (a federated identity is required). What the kit's private scenarios mean for mopm, honestly:

- Reads need no grant and carry no token. A grant that is presented is checked (repository, expiry, and
  coverage where the verb names coordinates) and is otherwise used only for the group lookup in the staged
  shape.
- **Version states.** `published` is `Present`. `yanked` is `Yanked(Held{sha256})`. `tombstoned` is never
  `Absent` and never `Present`: with a file `sha256` (a takedown, or a burned member that kept it) it is
  `Yanked(Held{sha256})`; with an empty file list (a burned member) it is `Permanent`, "the number is burned
  by a failed group, publish a new version". `fetch` of a tombstoned version is `None` (the record has no
  url). `newest` follows section 4. A 200 whose body lacks `sha256` for a published or yanked file is
  `Permanent` ("malformed"); the registry's own response schema requires it, so only a broken emulator or
  gateway produces it.
- **Declared set.** Staged shape: `observe(g, k, set)` with a non-empty `set` runs the group lookup keyed by
  that set (M1), then the package read, and combines them: package record found wins; otherwise a lookup
  that finds `k` staged with all files held gives `Staged(held)` where the digest is the declared one;
  otherwise `Absent`. Both reads use the same grant and no other call is made. A non-empty `set` lacking
  `k` is `Refused(Reason::Channel)`.
- **Empty set.** Only the package read is made. Staged state is never reported without a set, because the
  group is addressed by the declared set.
- **Another group's coordinate.** Any version published by any group or single publish reads `Present` with
  the digest mopm holds, which differs from ours when the bytes differ. It is never `Absent`.
- **Failed reads.** A failed lookup or package read fails the whole observe (`Transient`, or `Unknown` if
  the answer was lost). It is never read as "no group" or "not committed".
- One-call shape: only the package read exists; a non-empty `set` is checked for `k` and then ignored.
- A pending version held by another live group reads `Absent` (mopm hides pending rows). The following
  `commit` or `stage` answers `InFlight` or `Duplicate` as in section 6.
- **Digest trust.** The digest `observe` reports is the record's declared `sha256`, not a hash the publisher
  computed from fetched bytes. It equals the stored bytes because the upload carries a signed
  `x-amz-checksum-sha256` that storage verifies, and the validator hashes the file again at copy time. The
  trait doc of `observe` speaks of deciding by fetching and hashing; the publisher uses the record for
  `observe` and hashes bytes only in `fetch`. This is a trust argument and is noted as one.

The kit scenarios that need a read that is refused without a grant (private channel) cannot be satisfied by
a public registry. This is gap G-private and question Q1: the rig must be able to declare a shape without
`private`, or the kit must gate those scenarios on a declared capability.

## 9. Grants

`Grant` is opaque and carries a secret and an expiry. mopm has one token scope that matters (`publish`, for
exact `(name, version)` pairs) and no phases, so the stage, commit and read phases are mapped as follows.
Every token is publish-capable for its exact pairs, for stage, commit and yank alike; the phase is a label
in the envelope, not something the server checks.

**Threat model.** Phase separation here guards the engine against its own bugs (a stage grant used to
commit, a read grant used to write). It does not stop a holder of the token, who can call mopm directly.
Server enforcement is gap M10, which is ranked first with M1. Until then the grant source mints the
shortest-lived token per phase and never reuses one across phases, and a Stage envelope is handed only to
trusted code.

Which envelope authorises which verb, per shape (the shape is a construction parameter, so the publisher
checks the table for its own shape):

| Phase | Identity presented | One-call shape allows | Staged shape allows |
|---|---|---|---|
| Stage | publish-scope token, exact pairs | not used | create, upload, abort, lookup |
| Commit | publish-scope token (a fresh exchange) | create, upload, abort of own group, commit, poll, yank, lookup | commit, poll, yank, lookup |
| Read | `Grant::none()` always | package reads only | package reads only |

A test checks that a one-call commit under a Commit envelope sends create, and that the same Commit envelope
in the staged shape refuses create.

The grant secret that the mopm grant source produces and the publisher consumes is a length-framed envelope
in the same crate (module `grants`), so the two cannot drift:

```
version | phase | exceptional | registry location | length-framed list of the five coordinate fields | token
```

The envelope is private to the crate; the secret is never logged and the type stays `!Debug`. It is
unauthenticated: the publisher trusts the grant source that built it.

Rules, each checked client-side first and refused as `GrantRefused(...)` with nothing sent:

- **Presented grant, every verb.** `observe`, `fetch`, `newest` and `list` also check a grant that is
  presented, so a foreign-repository or expired grant on a read is refused and sends nothing (kit scenarios
  18 and 20 run on reads too). Repository and expiry are checked for every verb. Coverage is checked only
  where the verb names coordinates: for `observe` it is `k` plus every coordinate of the declared set (the
  lookup reveals the state of the set); `newest` and `list` name none and need an empty coverage list.
- **Read is `Grant::none()`.** No exchange is ever made for a read. A Read envelope that carries a token is
  malformed and refused. Validations, which may run arbitrary code, receive a Read grant only; a test checks
  that no token reaches a validation.
- **Repository binding.** The envelope's registry location is compared byte for byte with the configured
  registry. Mismatch: `Denied`.
- **Phase.** Per the table. A Read envelope or `Grant::none()` cannot write. Wrong phase: `Denied`. This is
  discipline, not server enforcement (gap M10).
- **Coverage per coordinate.** Every coordinate of the call must appear, by all five fields, in the
  envelope's list. One missing coordinate, not just the first, is `Denied`. The server independently binds
  the token to exact `(name, version)` pairs, at most 100 distinct package names (`grant_limit`).
- **Expiry.** A grant whose `expires_ms` is at or before the injected clock's now is `Expired`, checked
  before every authenticated request and not only at call entry. A 401 `token_expired` from mopm is
  `Expired` before commit is sent and `Unknown` after. An unknown or revoked token is `Denied`. The engine's
  margin logic is the engine's; the publisher checks the hard bound. A one-call `commit` additionally
  refuses to start unless the remaining lifetime covers the configured upload and poll budget, answering
  `Expired` with nothing sent.
- **Token lifetime.** The exchanged token lives 10 minutes, and a one-call `commit` can outlive it. Uploads
  use presigned URLs and need no token, but commit, poll and the cleanup abort do. A token that expires
  mid-poll yields `Unknown` and the engine resolves it with `observe`. A cleanup abort that fails for the
  same reason leaves the group open, holding reservations until it is resumed, superseded or expires (12
  hours); that is expected and resolved by resume. This is gap M2.
- **Identity forms.** A write capability needs the federated push identity and a CI identity of a form mopm
  allows for the stage in use (section 3: `ci_owner` on beta, `acts_for` on production). That is a setup
  fact the manual run checks.

### 9.1 Staged state integrity

The group id is a function of the actor and the set, and a create of a different set that shares a member
supersedes the actor's own open group. With an account-acting form, every job of that account is one
actor, so a pull-request run, a re-run, or an exceptional run can supersede or reopen the release run's
staged group. `acts_via` also lets any token of the account see every group of the account, narrowed only by
package. Therefore: stage runs only from the protected release identity; exceptional and normal runs of a
machine do not share members; the envelope carries `exceptional` so a log shows it. A superseded or expired
staged group is an expected failure of the staged shape and is answered `Duplicate` or `Unknown` (never
`Permanent`) so that the engine observes and stages again.

### 9.2 The mopm grant source

Section 9 says "the grant source" produces the envelope. For mopm it is a second component, `MopmGrantSource`
(PR 4), in the same crate so the codec has one owner. It exchanges one identity token per phase, because the
identity token's `jti` is single use, and never retries an exchange with the same identity token: a lost
exchange response burns it. `GrantFault::Used` (the exchange says the identity token was already used) and
`GrantFault::Unreachable` (the exchange could not be reached) are mapped in section 10. The real exchange
(pairs, 10 minutes, single-use `jti`) is exercised only by the manual run (PR 7); rows 18, 19 and 20 in
section 12 prove the envelope logic and nothing about the real exchange.

## 10. Transport and errors

HTTP client: `ureq` (blocking, small dependency tree, matches the synchronous `Publisher` trait) with
rustls. Constraints from `deny.toml`: no `openssl-sys`, no `native-tls`, permissive licences only, no
wildcards. The crypto provider is `ring` (Apache-2.0 AND ISC) and roots come from the platform's trust
store via `rustls-native-certs`, or from an explicit bundle given in `Config`; `webpki-roots` is not used
because its licence is not on the allow list. Both choices are checked by the licence and ban step of
`cargo xtask check` in PR 1; if `ring` is refused the fallback is to ask for an allow-list change, not to add
a C TLS stack. Archive handling uses the pure Rust `zip`, `tar` and `ruzstd` crates. The HTTP client choice
is the open decision in the release design; this crate is where it lands.

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
- Platform roots follow the platform loader, which can read the usual certificate-directory variables. That
  is the one environment influence this crate cannot remove while using platform roots; the supported way to
  pin trust is `Roots::Bundle(path)` in `Config`. This is Q10.
- **Secrets are redacted.** A `Secret` header value type and a URL type that are neither `Debug` nor
  `Display` carry the bearer token and the presigned URL's query signature. Every error message, log line and
  the emulator's request log redact the query string and the `Authorization` value. The bearer is borrowed
  from `Grant::secret()` and the header is built in a zeroizing buffer. A test fails if a canary token or
  signature appears in any `SeamError` message, log line or `Debug` output of a `Request`.

Configuration (a plain struct built by the caller from flags; no environment variables, and no
`MOPM_TOKEN`: the only credential is the grant), with no default registry: registry base URL, file-host
origins, roots, proxy, request timeout, connect timeout, retry count for idempotent calls, poll interval,
commit wait deadline, upload budget, reserving-repeat bound, `newest` page and version caps, ticks per
second for `Retry-After` (the seam's `Pause` definition, Q8).

Retries: create and commit are sent once per call (a repeat of create with the identical body is allowed
only for `reserving`, and to refresh expired upload URLs). `PUT` of file bytes and every `GET` retry on
connect failure, timeout, 500, 502, 503, 504 up to the configured count. An upload URL lives at most 300
seconds, and a storage 403 from an expired URL cannot be retried with the same URL: the publisher repeats the
identical create (200, fresh URLs for the files not yet stored) once and retries the `PUT`. To avoid it, a
file's `PUT` starts only if the create answer is younger than half the URL lifetime, else the create is
repeated first. `429` is never retried inside the publisher: it answers `RateLimited` and counts against the
engine's attempt bound.

Error mapping (the code is `error.code` of the mopm body; the HTTP status alone is never used to decide):

| Condition | `SeamError` |
|---|---|
| client-side grant check fails (repository, phase, coverage, malformed envelope) | `GrantRefused(Denied)` |
| client-side or server expiry before commit was sent | `GrantRefused(Expired)` |
| 401 `token_unknown`, `token_revoked`, `token_missing`; authorization refused | `GrantRefused(Denied)` |
| exchange: identity token already used | `GrantRefused(Used)` (never retried with the same identity token) |
| exchange: could not be reached | `GrantRefused(Unreachable)` |
| 429 | `RateLimited(Backoff{ticks})` from `Retry-After` |
| 503 `read_only_mode`; 404 `route_not_found` (create or read: no group publish, wrong base URL); a group-create refusal on a deployment with groups off; name, version, platform, file name, type or channel outside what mopm can hold (client check) | `Refused(Reason::Channel)` |
| 404 `package_not_found` or 403 `not_owner` on create or yank (the package is not claimed or not owned by the CI identity); 403 `name_reserved`; `limit_exceeded`; 413 `payload_too_large`; 400 `invalid_request` for the daily allowance | `Refused(Reason::Channel)` with a message that names the setup step; nothing landed |
| 404 `package_not_found` or `version_not_found` on a read | `Absent` (observe), `None` (fetch), empty (newest) |
| any other 404, a 404 with an unparsable body, 400 `invalid_name`, `invalid_version`, `upload_invalid` on create | read: `Transient`; create: `Permanent` (the client grammar drifted from mopm; fix the check) |
| 409 `version_exists` on create | `Duplicate` (section 5 rule for a sealed version) |
| 409 `group_conflict` on create | by the section 6 table |
| 409 `group_commit_refused`, `invalid_transition`; digest mismatch on fetch; malformed or unexpected body; an invalid archive | `Permanent` (`group_commit_refused` on a dead group first reads the version record, section 7) |
| group `rejected` or `failed` during commit | `Permanent` |
| 503 `unavailable` on create | `Transient` |
| any transport failure, 5xx, or unreadable answer on a mutating call after commit or yank was sent | `Unknown` |
| failure before commit is sent that changed nothing visible | `Transient` |
| any failure of a read | `Transient` |

`Permanent` is never used after a request that may have landed; the tests prove this with an emulator that
applies the commit and then drops the response.

## 11. The rig

CI cannot reach a live registry. ucdk's workflows run one `cargo xtask <verb>` and the repository is Rust
only, so a TypeScript mopm cannot be a CI dependency. The rig is therefore:

- **`ucdk-release-mopm-emulator`**, a pure crate implementing `Transport` in process: a model of the mopm
  subset above with an injected clock, deterministic ids and a request log. It does no I/O, so it can be a
  dev-dependency of the publisher and stay pure. It models what the sections above depend on, not only the
  happy path:
  - packages that must be claimed and owned (`package_not_found`, `not_owner`);
  - groups with every status, including a **validation delay** (the group is `validating` and package reads
    show `Absent` for a configurable number of ticks) so that the lost-answer scenarios run through that
    window, `rejected` (rows freed, `Absent`) and `failed` (every member a tombstone, a burn);
  - `skip_identical` on a repeat create of a past-open group, supersede of an own overlapping open group,
    the open-group and supersede caps, and every `group_conflict` message of the section 6 table;
  - 404 kinds, tombstoned version records, signed-checksum PUTs, yank, token checks, a 429 with
    `Retry-After`, and the `reserving` answer.
- **A payload hook.** mopm commits only valid package archives. The rig supplies
  `payload(coordinate, seed) -> Blob`, which returns a valid `mojo-<name>-<version>-<build>.conda` archive
  with an embedded `lib/mojo/<name>.mopm.json` and a content controlled by the seed (two seeds give two
  different archives for one coordinate). Every commit scenario takes its blob names and bytes from this
  hook; a kit that supplies only literal `Blob { name, bytes }` cannot drive this publisher, and the
  literals would end `Permanent` at the pre-check. This is a precondition (Q11). The kit's `Held` must not
  require the kit's own bytes to be what mopm re-hashes; it compares the digest the registry reports.
- **`MopmRig: PublisherRig`** in the publisher's `tests/` directory, built on the emulator.
  - `publisher(shape)` builds the publisher in the one-call or staged shape.
  - `arrange` maps to emulator faults: `HeldByAnother` (another actor publishes the version),
    `StagedByAnother` (another actor's open group holds it), `RaceNextCommit` (the version is taken after
    our reads and before our create; in the staged shape the rig takes it by killing our reservation, see
    Q5), `LoseAnswerAfterCommit` (apply, then drop the response, with the validation delay set),
    `LoseAnswerBeforeCommit` (drop the request), `FailAfterCommit` (apply, answer 503), `CommitSetFailsWhole`
    (validation rejects, and a second variant where the group `failed` and burns), `ListLags` (search omits
    a version), `FailGroupRead`, `FailIndexRead`.
  - `grant` and `foreign_grant` build envelopes; `advance` moves the injected clock; `reads` returns the
    request log mapped to Group and Index steps.
  - Rig step 1 claims every package of the scenario and sets its CI owner before any group is created. The
    emulator does it in process; the manual run does it with an operator-supplied claim.
- **Fidelity.** An emulator can drift from mopm. Three guards: (1) emulator responses are validated against
  mopm's published API description vendored into the emulator crate with its digest; (2) one
  recorded-transcript test per mopm error code the mapping table uses, including each `group_conflict`
  message, a bare 404 and a 404 with another code; (3) a manually run job (not in CI, not in any pull
  request) executes the same kit script against a non-production mopm deployment with an identity
  provisioned for that purpose. It starts only from a protected environment by manual dispatch, never from a
  fork pull request, creates the packages first (rig step 1), and checks before any exchange that the
  registry's own configuration carries an explicit non-production marker (gap M5), refusing with nothing sent
  if it does not. `Config` has no default registry. No production registry is ever a rig target. The manual
  run's checklist also records: the uncached strongly consistent version record, `Present` at `committed`
  with repodata lagging, whether a burned member keeps a digest, yank retention at the channel host, and
  the real token exchange. Gap M9 asks mopm to publish transcripts so guard (2) is generated, not hand
  written.
- **Transport adapter tests.** The `ureq` adapter is covered by a few tests against a `std::net::TcpListener`
  stub (timeouts, partial bodies, 429 with `Retry-After`, redirects refused, a foreign file origin refused).
  Everything above it runs on the emulator.
- Coverage: the 99% line and branch bar applies to the publisher and to the emulator separately.

Rows of section 12 marked "emulator only" are proven against this document's reading of mopm, not against
mopm. Scenarios 1 to 39 are proven against real mopm only for the rows the manual run repeats.

## 12. Scenario table

The rows follow the order of the kit's scenario list (the number is the position in that list); keyed by
name, so a reader holding the list maps each row to the right scenario.

Shapes: **OC** is the one-call shape, **ST** is the staged shape (needs M1). "N/A" means the verb is never
called for that shape (a non-staging channel never sees `stage` or `abort`); whether the kit skips those
scenarios for a non-staging shape is Q2. "GAP" names the gap in section 13. "Emulator only" marks a row whose
evidence is the in-process emulator.

| # | Scenario | OC | ST |
|---|---|---|---|
| 1 | observe_absent_is_not_an_error | `Absent` only for a parsed 404 `package_not_found` or `version_not_found`; a missing platform of a held version is `Absent` | same |
| 2 | stage_then_observe_returns_staged_with_the_same_digest | N/A | create+PUT, then lookup+read returns declared digest (M1) |
| 3 | a_staged_coordinate_is_not_readable_by_a_stranger | N/A | mopm hides pending rows from every read; `fetch` follows the record's url only |
| 4 | a_stage_grant_observes_the_staged_state_it_made | N/A | lookup with stage envelope (M1) |
| 5 | a_commit_grant_observes_it_too | N/A | same token class, commit envelope (M1) |
| 6 | commit_makes_the_staged_coordinates_readable_with_the_same_digest | commit to `visible`, `Present` with `files[].sha256` | same |
| 7 | stage_of_present_same_is_duplicate_and_changes_nothing | N/A | `version_exists` preflight, nothing reserved; GAP-S when the set shares a `(name, version)` with a missing sibling |
| 8 | commit_over_present_different_leaves_old_bytes | `Duplicate`, old bytes stand; GAP-S for a partly held version | stage gives `Duplicate` first; commit on a dead group gives `Duplicate` when the version is held; GAP-S |
| 9 | abort_gives_staged_state_back_and_never_touches_a_committed_coordinate | N/A | abort table of section 7; `committed` gives `Ok` without effect |
| 10 | a_set_commit_makes_every_coordinate_readable_or_none | one group; `rejected` leaves `Absent`; `failed` leaves the members burned (never `Absent`, section 7); emulator models both | same |
| 11 | a_channel_that_does_not_stage_commits_with_the_bytes | the shape itself, with archives from the payload hook | N/A |
| 12 | no_digest_exposed_is_refused | emulator only: real mopm requires `sha256` in every file reference, so a 200 without it is `Permanent` ("malformed") and only a broken emulator produces it | same |
| 13 | yank_keeps_bytes_fetchable | version yank; the channel specification states retention; the manual run proves it at the channel host (M11 narrowed) | same |
| 14 | yanked_coordinate_cannot_be_committed_again | `version_exists` gives `Duplicate` | same |
| 15 | lost_answer_is_resolvable_by_observe | `Unknown`, then `Present`, `Absent`, or during `validating` a retry that gives `Committed` (section 7); Q2: kit must accept `Absent` for a non-staging channel; emulator runs through the validation delay | `Staged` or `Present` |
| 16 | list_may_lag_but_observe_is_exact | list from search `latest`; observe from the uncached version record (Q6 answered from source, confirmed by the manual run) | same |
| 17 | public_read_needs_no_grant | all reads unauthenticated; write without a grant `Denied` | same |
| 18 | a_grant_for_another_repository_sends_nothing | envelope check on every verb including reads; request log empty; proves the envelope logic only | same |
| 19 | a_grant_of_the_wrong_phase_is_denied_and_nothing_happens | client-side phase check; proves the envelope logic only (a stage token and a commit token are both mopm `publish` tokens); GAP M10 for server enforcement | same |
| 20 | an_expired_grant_is_refused_and_nothing_happens | injected clock, checked on every verb; 401 `token_expired`; the real exchange is only exercised by the manual run | same |
| 21 | newest_lists_every_held_coordinate_of_the_name | versions list plus per-version record, yanked included, empty for no package, a failed fetch of any version fails the call, capped with an error | same |
| 22 | observe_returns_the_stamp_it_was_given | GAP M3 | GAP M3 |
| 23 | a_granted_observe_sees_a_coordinate_committed_by_another_writer | public read | same |
| 24 | a_private_observe_reads_the_group_then_the_index | GAP G-private: one read only | lookup then read, same grant (M1) |
| 25 | a_coordinate_committed_in_another_group_reads_as_present_never_absent | `Present` with mopm's digest | same |
| 26 | a_failed_group_read_or_index_read_fails_the_observe_and_is_not_absent | index half only; a bare 404 or a 404 with another code is `Transient` | both halves |
| 27 | observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set | N/A | lookup keyed by the declared set (M1) |
| 28 | observe_with_an_empty_set_cannot_see_staged_state_and_still_sees_committed_coordinates | package read only | same |
| 29 | a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit | read is `Grant::none()`; a Read envelope carrying a token is refused; write refused client-side `Denied` | same |
| 30 | commit_is_atomic_when_the_coordinate_appears_between_observe_and_commit | `version_exists` gives `Duplicate`, winner stands | race only during `stage`; see Q5 |
| 31 | transient_after_commit_is_observed_before_retry | `Unknown`, never `Permanent` | same |
| 32 | duplicate_over_other_bytes_leaves_the_old_bytes | `Duplicate`; GAP-S if the set shares a `(name, version)` | GAP M14 (a same-actor stage of other bytes supersedes) and GAP-S |
| 33 | a_second_commit_of_the_same_bytes_is_duplicate | create 200 `skip_identical` on a `committed` or `visible` group gives `Duplicate`; on a `validating` group the retry gives `Committed` (section 7) | body status `committed` or `visible` at entry |
| 34 | a_stage_over_ones_own_staged_coordinate_is_duplicate | N/A | identical set: create 200, every file held, `Duplicate`; other bytes: GAP M14 |
| 35 | a_commit_carries_exactly_what_was_staged | N/A | client compares items with declared digests from lookup (`Permanent`); server `group_commit_refused`; stamp part GAP M3 |
| 36 | a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was | preflight before reserve; `group_conflict` mid-reservation aborts the group server-side (nothing stays reserved); GAP-S | same |
| 37 | a_set_that_names_a_coordinate_twice_is_refused | client check, no I/O | same |
| 38 | a_stage_or_commit_grant_reads_no_staged_state_but_through_observe | `fetch`, `newest`, `list` are public reads and carry no token | same |
| 39 | a_channel_that_does_not_stage_creates_a_coordinate_once | lost race, yanked recommit, post-landing failure `Unknown`, failed set commit (rejected and burned): all covered | N/A |

Scenarios that exist only in the staged shape: 2, 3, 4, 5, 9, 27, 34, 35 and the staged halves of 7 and 36.
Scenarios blocked on a gap: 22 (M3), the private halves of 24 and 26 (G-private, M1), 32 and 34 in the staged
shape (M14), and 7, 8, 32 and 36 where a set shares a `(name, version)` (GAP-S, Q9). Rows proven on the
emulator only: 12, and every row whose evidence is a mopm behaviour the emulator was written to copy, until
PR 7.

## 13. Gaps in mopm

Owner for every row is the mopm maintainers. "Meanwhile" says what the publisher does until the change.
Priority order: M10 and M1 first, then M13, M3, M2, M14, then the rest.

| Id | Gap | Recommended mopm change | Meanwhile |
|---|---|---|---|
| M1 | No read-only way to find a group from its declared set and see what it holds (member states only; no digests; id derivation is not a documented interface, and the documented CLI keeps the group id in a local state file) | `GET` a group by `(actor, set digest)` or list the actor's open groups, owner-only, returning status, per-member per-file declared `sha256`, `size` and `uploaded` | Staged shape not built; ship the one-call shape. Never use create as a read. A local record of group ids is rejected (decision 1). |
| M2 | A 10 minute token cannot cover upload, commit and the validation wait; the CLI has no refresh | Group-scoped token that outlives the group's validation, or a refresh route | Refuse to start a one-call commit without enough lifetime; `Unknown` on expiry mid-poll, resolved by `observe` |
| M3 | No place to store a stamp (`source_commit`, `exceptional`); build number exists per file in repodata but is not in the version record | Immutable per-version annotation map set at create, returned by version detail | Declare the channel unfit for `monotonic` and for scenario 22; do not invent an encoding in the build string |
| M4 | (withdrawn) A conflict mid-reservation was believed to leave earlier members reserved. A preflight conflict reserves nothing, and a number taken afterwards aborts the group on the server | none; the next create is a fresh attempt and is charged against the daily allowance | documented in section 6 |
| M5 | No capability advertisement and no non-production marker | Add group publish, API version and the stage name to the index config | The shape is configuration; the manual run refuses a registry without the marker |
| M6 | A published version does not name its group | Return the group id in version detail | Group yank unused; yank is per version |
| M7 | Version list has no file digests, so `newest` is N+1 reads | Include `files[].sha256` in the versions listing | Cap and fail the whole call on any failed read |
| M8 | Commit needs `manifest_sha256`, which the client must extract from the archive | Let commit omit it and have the server fill it | The publisher parses the archive (`zip`, `tar`, `ruzstd`) in a small module; a malformed archive is `Permanent` before any I/O |
| M9 | No recorded wire transcripts for the emulator | Publish golden transcripts per error code | Hand-written transcripts plus the manual non-production run |
| M10 | One token scope for stage and commit; the token that staged can commit and yank | A stage-only scope (create, upload, abort; no commit, no yank) | Client-side phase check; documented discipline-only (section 9 threat model) |
| M11 | Not stated in the API description: that the version record is uncached at every layer and that yank keeps channel bytes (both are stated in source and the channel specification) | State both in the API description | Proven by the manual run at the channel host before the publisher is declared conforming |
| M12 | `version_exists` carries the first file's digest only | Carry every file's digest | The publisher reads the version record itself |
| M13 | `group_conflict` is one code for a held number, the open-group cap, the supersede cap, a failed group retried and an unreserved takeover; the body has no structured detail | Distinct codes (or a structured `reason` field) | Classify by message prefix, fail closed on an unknown message (section 6); transcripts pin each |
| M14 | A create of a different set by the same actor that shares a member supersedes the earlier open group silently | A no-supersede flag on create, or a refusal when the overlap is the same actor | Staged shape cannot answer `Duplicate` for other bytes without M1; rows 32 and 34 are GAP |
| M15 | A CI identity cannot claim a package, and a group create needs it claimed and owned | Allow a configured CI identity to claim a name on a non-production stage, or document the operator step | Rig step 1 and the manual run claim the package first |
| GAP-S | A version is sealed by its first group; a set naming an extra platform of a held version cannot be created. The seam asks for `Duplicate` whenever any member is held | Seam or kit decision (Q9), not a mopm change | `Refused(Channel)` on write, `Absent` on observe, no loop |
| G-private | No private visibility | Out of scope; a visibility concept is a registry product decision | Publisher refuses a PRIVATE repository; kit scenarios needing a refused read are gated (Q1) |

A spec drift to correct in mopm's API description: it says a CI identity "cannot create a group" while the
configured-publisher form can.

## 14. Decisions

1. **Ship the one-call shape first; build the staged shape behind M1.** Why: it works against mopm as
   deployed, and the alternative (observing staged state by re-sending create) violates `observe` being a
   read. Alternatives: refuse mopm until M1 (delays the first real channel for a dependency we do not own);
   keep a local record of group ids (mopm's own CLI does exactly this in its state file, but it is a second
   source of truth that is lost with the machine and fails the lost-answer scenarios).
2. **One crate, `ucdk-release-mopm`, containing the publisher, the grant envelope codec and the mopm grant
   source (PR 4).** Why: the envelope is a private contract between its halves. Alternative: a separate
   grant-source crate (two crates must agree on an undocumented format).
3. **`CommitUnit::Set`, `conditional_put() == true`.** Why: group commit is all-or-none and create is
   compare-and-set. Alternative `Coordinate` would break atomicity across platforms of one version.
4. **`observe` reads the package record, never `list` or search.** Why: seam rule; the record is the
   authoritative per-version read.
5. **A version sealed with another file set is `Refused(Channel)` on write and `Absent` on observe, never
   `Duplicate`; recorded as GAP-S and Q9, not as settled.** Why: avoids an observe-Absent, commit-Duplicate
   loop. Alternative: widen `Present` to the whole version (lies about a coordinate not held).
6. **`Committed` is returned at `visible`.** Why: the seam's "readable" then holds on every mopm read path.
   Alternative: return at `committed` (shorter call, but repodata can lag the answer).
7. **Yank is per version, documented, and refused when the version record holds coordinates the grant does
   not cover.** Why: mopm has no finer unit and a yank must not exceed its grant. Alternative: yank anyway
   (exceeds coverage).
8. **Phase, repository, coverage and expiry are enforced client-side, on every verb.** Why: "nothing is
   sent" is a client-side promise anyway, and the server has one scope. These rows prove envelope logic
   only. Alternative: wait for M10 (the kit's scenarios 18 to 20 would have no real implementation).
9. **`ureq` with rustls, `ring` and native roots or an explicit bundle; blocking; an injectable
   `Transport`.** Why: matches the synchronous trait and the deny list. Alternatives: `reqwest` (drags an
   async runtime); `hyper` directly (more code to cover at 99%).
10. **Emulator in process, pure, as the rig; a manual non-production run for fidelity; no prod.** Why: CI
    is Rust only and never reaches a live registry. Alternative: drive mopm's own server from CI (adds a
    TypeScript runtime to CI).
11. **Manifest hash computed client-side from the archive.** Why: not blocked on M8. Alternative: wait for
    M8.
12. **`grant_limit()` is 100 coordinates; a set with two versions of one name is refused.** Why:
    conservative against the 100 distinct names limit. Alternative: count distinct names (needs a seam
    change, Q4).
13. **A PRIVATE or UNSPECIFIED repository is refused at construction, and an anonymous push identity
    cannot write.** Why: mopm cannot honour private reads and an anonymous write has no token.
14. **No environment variables.** The registry, timeouts and caps are configuration; the credential is the
    grant. The one exception is platform root loading (Q10).
15. **`group_conflict` is classified by message prefix and fails closed.** Why: the code is ambiguous
    (M13) and a wrong `Duplicate` or `InFlight` loops the engine. Alternative: map it wholesale (loops on a
    cap error).
16. **A 404 is `Absent` only with a parsed `package_not_found` or `version_not_found` body.** Why: a
    mis-pointed registry must not look empty.
17. **A tombstoned record is never `Absent`.** `Yanked` with a digest, `Permanent` without. Why: a burned
    number can never be committed again.

### Review dispositions

Minor findings from the design review, taken or rejected:

- Taken: the questions answerable from source are answered (Q6, Q7); Q8 refers to the seam's `Pause`.
- Taken: `newest` empty, tombstoned and partial-failure cases; yank tightening (280 characters, 404, the
  retention citation); the upload URL lifetime; cross references fixed; file and function citations in
  section 3; the mopm commit read is named.
- Taken: scenario rows are numbered in the order of the kit's list and keyed by name.
- Taken: the quarantine-readability claim is dropped (section 6).
- Rejected: using `GET /packages/{n}/versions` as the `newest` source today. `GET /packages/{n}` is one
  unpaginated read and the cap makes an over-large list an error; the paged route is kept as an option if
  M7 lands. Why: fewer requests per call, no cursor state.
- Rejected: asking the kit for an "emulator only" status for row 12 as a separate scenario kind. The row
  stays in the table, marked.

## 15. PR plan

Each PR is small, passes `cargo xtask check` and the 99% line and branch coverage bar for the crates it
touches, and ships tests that failed first for any fix.

1. **Skeleton and mapping types.** Crate manifest (not `pure`), licence headers, `Config` (no default
   registry), `Transport` trait and types, `Secret` types, coordinate-to-member mapping with mopm's exact
   grammars, set checks, error mapping table, the `ureq` dependency checked against `deny.toml`. Tests:
   mapping and injectivity (coordinates differing only where a `/` falls), grammar refusal for observe too,
   an invalid-grammar version never produces a request, every row of the error table.
2. **One-call `commit`, `fetch`, `yank`.** Create, upload, commit, poll; the create-answer classification
   (including 200 `skip_identical` and the `group_conflict` table); manifest hash extraction module;
   `Unknown` after send. Tests on a hand-built fake `Transport`, including apply-then-drop, the `validating`
   window with `observe` giving `Absent`, and the cap errors never giving `Duplicate`.
3. **`observe`, `newest`, `list`.** Package read, 404 kinds, tombstoned mapping, sealed-version rule,
   ambiguity rule, caps. Tests for lag, failed reads never `Absent` (a bare 404 first), a failed fetch of the
   third of five versions, no digest.
4. **Grants.** Envelope codec, repository, phase (per shape), coverage, expiry checks on every verb with an
   empty request log on refusal; the 401 mappings; `MopmGrantSource` (one identity token per phase, `Used`
   and `Unreachable`).
5. **Emulator and rig; kit.** `ucdk-release-mopm-emulator` (section 11), `MopmRig` and the payload hook,
   `run_publisher_kit` for the one-call shape, plus the TCP-stub adapter tests. Lands after kit parts 6 and 7.
6. **Staged shape** (after M1): lookup, `stage`, `abort`, staged `commit`, `observe` with a declared set;
   extends the emulator and runs the staged shape through the kit.
7. **Manual non-production conformance job** script and its documentation: protected environment, manual
   dispatch, package claim step, non-production marker check, and the checklist of section 11. Scenarios are
   marked proven against real mopm only after this run.

## 16. Open questions

For the kit and seam owners (must be answered before the publisher kit parts open, because a seam lands
whole):

- **Q1.** Can the rig declare a shape that is not `private`, or are the private scenarios (23 to 28 and the
  read-needs-grant parts) gated on a declared capability? A public registry cannot satisfy a read that must
  be refused without a grant.
- **Q2.** For a shape with `stages() == false`, does the kit skip the staging, abort and `Staged`
  scenarios, and does lost-answer resolution accept `Absent`, and a retry that answers `Committed` for a
  group still validating?
- **Q3.** May `yank` of one coordinate widen to every coordinate of the same `(name, version)`? mopm's unit
  is the version. The publisher refuses when the grant does not cover the siblings.
- **Q4.** `grant_limit` counts coordinates; mopm's limit counts distinct package names. Is a conservative
  100 acceptable, or should the seam count distinct names?
- **Q5.** `RaceNextCommit` in the staged shape: after `stage` the reservation prevents a race. Is the rig
  allowed to realize it as "our group lost its reservation" and expect `Duplicate` (version now held) or
  `Permanent` (not held)?
- **Q6.** Answered from source: the version record is uncached and strongly consistent at the origin, and
  `Present` flips at `committed` while repodata lags. The manual run confirms the deployment (gateway
  layer). Is `Present` at the package record's commit moment acceptable?
- **Q7.** Answered from source: held digests are lowercase 64-hex SHA-256, the same form as
  `Hash256::as_str`. Kept in the manual-run checklist.
- **Q8.** Unit of `Backoff.ticks`: is it the `Pause` the seam defines? The publisher converts `Retry-After`
  seconds with the configured ticks per second.
- **Q9.** Does the kit ever build a set from two platforms of one `(name, version)`? If so, either vary the
  name, or agree that `Duplicate` plus an `observe` that reports the missing sibling is the answer for a
  partly held version (GAP-S).
- **Q10.** May platform root loading, which can read certificate-directory variables, be the one exception to
  "no environment variables", with `Roots::Bundle(path)` as the pinned alternative?
- **Q11.** Does the kit take blob names and bytes from a rig-supplied `payload(coordinate, seed)` hook, so
  that two seeds give two valid archives for one coordinate, and a `Held` compare uses the digest the
  registry reports?
- **Q12.** A burned coordinate (a member of a `failed` group) has no digest. Should the seam gain a
  `Burned` presence, or is `Permanent` from `observe` acceptable?

For the mopm maintainers: M1 through M15 above, in priority order M10 and M1, then M13, M3, M2, M14, then
the rest.
