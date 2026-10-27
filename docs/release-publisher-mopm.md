# The mopm release Publisher

Status: design, revision 5. Nothing in this document is implemented yet. Where it says "the publisher does X"
it describes what the crate will do when the PRs in section 15 land. The document is in two files to stay
under the size bound: this one holds sections 1, 2, 4 to 9 and 14 to 17 (the design); sections 3, 10, 11, 12
and 13 (the mopm facts, the transport and the error table, the rig, the scenario table and the changes asked of
mopm) are in [release-publisher-mopm-verification.md](release-publisher-mopm-verification.md). Section numbers are global.

Revision 5 follows the maintainers' direction: publishing is the pattern every package manager has, stage,
validate, publish, with abort, and the design, the seam and the engine express that shape first. A channel
that cannot stage is the special case. The earlier two-shape design (a one-call shape first, the staged shape
behind mopm changes) is gone: mopm stages, and the changes it needed have landed or are in review. What
changed is listed in section 16; what this changes in the seam and the engine documents is section 17.

## 1. Summary

ucdk's first real release channel is a mopm registry. This document designs `ucdk-release-mopm`, a Rust
crate that implements `ucdk_release_seams::Publisher` against mopm's HTTP API (the group publish path and
the public read path) and is held to the publisher kit (`run_publisher_kit::<R: PublisherRig>`, 39
scenarios, nine contract points, and the scenarios the seam owners added for the kit's last two parts).

**The shape.** A release is published in three steps with an abort, each its own seam call and each bounded
on its own:

1. **Stage.** `stage` puts the whole set on the registry in a place nobody can read: bytes uploaded, numbers
   reserved, nothing visible.
2. **Validate.** The engine runs the tests and checks of the Publish action on the staged set before anything
   is public. mopm exposes no read of staged bytes, so the engine validates its own local copy of the set and
   proves it is the staged one by digest: the registry reports the declared `sha256` of every staged file,
   which storage verified at upload, and the engine requires it to equal the digest of the member it
   validated (section 7.1).
3. **Publish.** `commit` makes the staged set readable, all or none. `abort` gives the staged state back, and
   the engine calls it when validation fails.

`stages()` is `true`. The shape is not a mode of this crate: it is the seam's primary shape, and a channel
that cannot stage (a directory, a plain tag push) is the special case, declared by `stages() == false`
(section 17). There is one code path per verb.

Four facts shape everything:

1. mopm has a server-side staged set: a *group*. It is open and invisible, it is create-if-absent per
   `(name, version)`, its commit is all-or-none, and an open group can be aborted. `mopm publish --staged
   <dir> --no-commit` opens a group, uploads, and leaves it open and invisible, and the project's own release
   flow already runs stage, test the exact staged bytes locally (verified against the plan), commit, and
   abort on failure. That is `CommitUnit::Set`.
2. The staged state can be observed without a write. The group id is a documented function of the actor and
   the declared set, with test vectors (mopm#261), so the publisher computes it and reads
   `GET /groups/{id}`, which lists each member's declared files (`subdir`, `filename`, `sha256`, `size`,
   and `uploaded` while the group is open); 404 means no group. `getVersion` names the group of a version, and
   `manifest_sha256` is optional on commit. Since mopm#260 (live) a `group_conflict` carries a stable
   `error.reason`, create takes `on_overlap: supersede | refuse`, and the answer lists the groups it
   superseded. mopm#261 is in review: the staged shape's observe is built against its documented contract and
   proven on the emulator now, and against a real registry after it is deployed (section 13).
3. mopm cannot be asked for the bytes of a staged file. Validation reads the engine's local staged set, and
   the read-back after the commit fetches the published bytes and the engine hashes them (section 8).
4. mopm seals a `(name, version)` once a group commits (and burns it if the group fails). A later set cannot
   add a platform to a version. A release for several platforms must declare its whole platform set before
   the first group that can commit (section 5.1), and every platform archive of one version must agree on one
   manifest. The maintainers rejected a change here; the engine and the release machine must adapt.

mopm is a public registry for Mojo library packages. It has no private visibility, no per-phase scopes, no
place to store a build stamp today, and it accepts only valid Mojo library archives of a fixed name grammar.
Section 13 lists every gap with the maintainers' verdict, the agreed order, and what the publisher does
until each lands (refuse, or degrade with the degradation named). Section 12 says, scenario by scenario,
which are met, which are met after a named mopm change, which are skipped by the shape the publisher
declares, and which are a gap.

Three things in this document are stated as limits of the evidence, not as facts about the product:

- Section 3 was read from the mopm sources and API description, not executed, and the facts that come from
  mopm#261 were read from its description and the specification text it adds; it is not merged.
- The in-process emulator (section 11) copies this document's reading of mopm. A scenario that passes on it
  is evidence about the publisher, not about mopm, until the manual run of PR 7 repeats it against a real
  deployment.
- The kit parts for the publisher (4g, 4h) have not landed. Their hooks are those the seam owners decided
  (section 11); a row that depends on a hook says so.

## 2. Goals and non-goals

Goals

- Express stage, validate and publish, with abort, as the primary shape: `stages()` is `true`, `stage`, `commit`
  and `abort` are separate calls each bounded on its own, and the staged set can be proven to be the one that
  was validated (section 7.1).
- Implement every `Publisher` method with the semantics in `publisher.rs`, and pass the kit for the shape the
  publisher declares.
- Never decide an effect on a lagging read. `observe` reads the authoritative per-version record; `list` is
  used for reports only. No verdict of "published" rests on a digest the registry reports: the read-back
  fetches the bytes and the engine hashes them (section 8).
- A mutating call whose transport failed answers `Unknown`; a call after which an effect may have landed
  never answers `Permanent` or `Refused`.
- Refuse, at construction or preflight, what mopm cannot honour, instead of faking it. Refuse before any I/O
  whatever mopm would answer with a 400, so that no request that can never succeed is ever sent or retried.
- Client-side enforcement of grant repository, phase, coverage and expiry, so a refused call sends nothing.
- Bound each call below the lease and the token it runs under, checked per call at construction (section 9.3).
- All timeouts, retry counts, page caps and the registry location are configuration passed in by the
  caller, never environment variables. Trust roots are one explicit bundle file named by a flag.

Non-goals

- The OIDC exchange itself. It belongs to the grant source (section 9 gives the envelope both sides share,
  and PR 4 builds the mopm grant source that produces it).
- Private repositories. mopm has none; the publisher refuses a PRIVATE repository (section 8, G-private).
- Claiming a package name. The publisher never claims one (section 3 says how a package comes to exist).
- Reaching a live registry from CI. CI runs the kit against an in-process emulator (section 11).
- Changing the seam, or mopm, from this repository. Seam changes are decided by the seam owners (section 17
  asks for a comment and nothing else); mopm changes are requests (section 13).

## 4. The mapping

`Publisher` capability getters:

| Getter | Value | Why |
|---|---|---|
| `types()` | `[CONDA]` | a `.conda` file is a conda artifact; `CONDA` names an identity (the SHA-256 of the file's bytes, which is mopm's `files[].sha256`) since ucdk#42. A coordinate of any other type is `Refused(Channel)`. |
| `stages()` | `true` | mopm holds a staged set in an open, invisible group, so the channel has the stage step. This is the seam's primary shape; `false` is the declared exception for a channel that cannot (section 17). |
| `commit_unit()` | `Set` | a group commits all or none |
| `conditional_put()` | `true` | create refuses an existing version (`version_exists`), commit is compare-and-set on group status |
| `grant_limit()` | `100` | counted in coordinates, as the seam defines it; mopm's limit counts distinct names, so no set of 100 coordinates can exceed it. It refuses at most four times too many legal sets (four subdirectories). |
| `yank_unit()` | `Version` | mopm yanks a `(name, version)`: every platform file (section 7) |

The binding limit for a large tree is the 100 distinct names per token, however it is counted: a release of
more than 100 packages must be several Publish actions (a set is one action). That is a release-machine
decision, and the two limits agree on it.

Method mapping. "Public" means no token. "Token" means the grant's secret as bearer, attached only to the
calls that say so and never to a read or a file URL. The group id is computed by the publisher from the actor
and the declared set (section 3) and is never a stored value.

| Method | mopm call | Grant used | Error mapping (summary; full table in section 10) | Idempotency and atomicity argument |
|---|---|---|---|---|
| `observe(g, k, set)` | With a non-empty `set`: the group read of the group id computed from the actor and `set` (token), then the package read `GET /packages/{n}/{v}` (public). With an empty `set`: the package read only. | A grant that is presented is checked for repository and expiry on every verb, and for coverage where the verb names coordinates (section 9). The group read needs a stage or commit envelope. | A 404 whose parsed body code is `package_not_found` or `version_not_found` gives `Absent`; for the group read only a 404 `group_not_found` means "no group". Any other 404, an unparsable 404, and every read failure is `Transient` (`Refused(Channel)` for `route_not_found`), never `Absent`. A tombstone is `Burned` or `Yanked` (section 8). An expired grant that is presented is `GrantRefused(Expired)`, which `SeamError::observe_failure` treats as `INDETERMINATE` even at preflight. | A read. Exactness rests on the version record being uncached and strongly consistent (section 3, proven against a deployment by the manual run). Never calls `list`, search or create. Reports the registry's own digest (for a staged file, the declared one that storage verified at upload); no verdict rests on it alone (section 8). |
| `stage(g, items)` | the group read first: open with every declared file uploaded gives `Duplicate`; else `POST /groups` with `on_overlap: refuse`, repeated with the identical body while `reserving`, then `PUT` each file the answer lists with `uploaded: false`, with the returned headers and no token | stage grant | 409 `version_exists` gives `Duplicate` (a burned coordinate too: the engine's observe reads `Burned`); every `group_conflict` is classified by its reason (section 6) | The group id is a function of actor and declared set, so a repeat of the identical set resumes the same group. A different set that shares a member is refused, never superseded; a create that lists a superseded group anyway is "not honoured" (section 6). |
| `commit(g, items)` | `POST /groups/{id}/commit` naming each member and sending no manifest hash, then poll `GET /groups/{id}` until `visible`. The answer is decided by the group status in the body, not the HTTP status. | commit grant | body status already `committed` or `visible` gives `Duplicate`; `validating` is polled and gives `Committed` at `visible`; a refused commit is classified by a status read (section 7) | an identical commit body is idempotent server-side; a different body is 409 `group_commit_refused`. Sent once. |
| `abort(g, ks)` | `DELETE /groups/{id}`; on 409 or 404, `GET /groups/{id}` | stage grant | section 7 table | DELETE is idempotent only for `aborted`; the follow-up read makes the rest idempotent |
| `yank(g, k, reason)` | read the version record (`k` must be held in it, else `Permanent`), check the grant covers every held sibling, then `PUT /packages/{n}/{v}/yank` with `{yanked: true, reason}` | commit grant covering `k` and every held sibling of its version | see section 7 | server-side no-op on repeat. Coarser than a coordinate: `yank_unit() == Version`, so the engine names and covers every sibling (section 7). |
| `fetch(g, k)` | `GET /packages/{n}/{v}`, then `GET` the file `url` from that record for `k`'s subdirectory | none | 404 version, no such file, a tombstoned version (no url) give `None` | Returns the bytes as served, and judges nothing: the engine hashes them against the set's digest. Never builds a URL itself, and a staged file has no URL, so staged bytes are never returned. The URL's origin is checked against configuration (section 10). |
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
  `Refused(Reason::Channel)` before anything is sent. A blob that is not a valid package archive (it has no
  embedded manifest) is `Permanent` before any I/O: the registry would reject the group, and a group spends the
  daily allowance per attempt. Also checked before any I/O, each a
  `Permanent` or `Refused` so that no daily allowance is spent on a group mopm would reject: a version over
  128 characters, an empty file, more than 500 members or a file named twice, a group over 20 GB in total, and that every archive of one member
  embeds the same `lib/mojo/<name>.mopm.json` bytes and agrees on build and native library (section 5.1).
  The commit sends no manifest hash (mopm#261), so the client's archive reading is for these refusals only.

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

## 6. Stage: a mopm group

- `stage` computes the members (one per `(name, version)`, files sorted), computes the group id from the
  actor and the declared set (the derivation mopm#261 documents; pinned here by its test vectors as golden
  tests), and reads the group (`GET /groups/{id}`). 404 `group_not_found` is "no group". A group that is open
  with every declared file `uploaded` is `Duplicate`, with no create sent. Otherwise it sends `POST /groups`
  with `on_overlap: refuse` and repeats the identical request while the answer is `reserving` (bounded by
  configuration), then uploads every file the answer lists with `uploaded: false`.
- **The id the answer names must be the one computed.** The create answer carries the group id. A different
  one means the derivation (or the actor string) drifted: the publisher aborts the group the answer names, best
  effort, and answers `Permanent` ("group id derivation drifted") when the abort succeeds and `Unknown` when it
  does not. The manual run compares the two against the documented vectors.
- **`on_overlap: refuse` is always sent, and what can be checked is limited.** An old registry ignores the
  field, supersedes silently and names nothing in `superseded`; a registry that honours it never lists a
  superseded group under `refuse` and, since mopm#260, answers `overlap_refused` for an overlap. The create
  answer has no positive marker that the field was honoured (`superseded` is absent when nothing was ended), so
  the publisher does not pretend to detect an old registry from an answer. It checks the one thing an answer
  can show, a `superseded` list under `refuse`, which contradicts the contract: the publisher answers
  `Refused(Channel)` ("registry does not honour on_overlap") for that stage and every later one of the same
  publisher. The registry's level is a requirement proven by the manual run, which creates an overlapping
  second set and expects `overlap_refused` (section 11), and a positive marker is asked of the maintainers
  (M16, section 13). Until a registry has passed that probe the exposure is a same-actor open group of another
  set losing its staged bytes, which the engine finds at the commit (`PUBLISH_STAGE`), never silently.
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
- **`group_conflict` is classified by `error.reason` and by nothing else.** Since mopm#260 it carries a stable
  reason from a closed list. The message is free text inside a fixed wrapper and the publisher parses none of
  it, so a wording change in a mopm release cannot change a class. A client must treat an unknown reason like
  an absent one: `Unknown`, and the engine goes to its read-back, which polls `Publish.polls` times (default
  3, one tick apart); the action is `INDETERMINATE` when those are spent. `Duplicate` is never answered for a
  conflict the table does not name. The classes:

| `error.reason` | `stage` answers |
|---|---|
| `number_held` (a live group holds a number: another account's, or one of our own that is validating or committed) | `Duplicate`: held by a group that stands; the engine's observe reads `Absent` and ends `INDETERMINATE` (it cannot see another group's staged state), never a retry loop |
| `overlap_refused` (our own open group of another set holds it; nothing was ended) | `Duplicate`: the first staged bytes stand |
| `open_cap` (5 open groups per actor) | `Refused(Channel)` ("abort an open group") |
| `supersede_cap` | `Refused(Channel)`; under `refuse` the registry reports `overlap_refused` first, so this is not expected |
| `group_failed` (numbers burned) | `Transient` ("burned; publish a new version"); the engine's observe reads `Burned` |
| `takeover_blocked` (a reserved row not yet free) | `Transient`: nothing was reserved, the same call may succeed later |
| `number_taken` (another publish took a number mid-create; the server ended our group) | `Duplicate` if the version record now shows the coordinate held, else `Transient` once |
| no reason, or one not in this table | `Unknown` |

  A caller over its open-group cap is never told `Duplicate`, so the engine cannot loop. Each row is pinned
  by an emulator case and, once mopm publishes transcripts (M9), by a golden. A burned set re-created by
  the same actor reaches `group_conflict` before `version_exists`, so it is the `group_failed` row, never
  `Duplicate`; a burned number reached by a different set is `version_exists`, `Duplicate`, and the
  engine's observe reads `Burned` (the seam refuses it as `PUBLISH_BURNED`, never retried).
- Create is sent once per attempt and never blindly retried after an `Unknown`; the engine observes first.
  Two exceptions, each bounded: the identical create while `reserving`, and a re-create to refresh upload
  URLs (section 10).
- `observe` must not call create. Create reopens dead groups, charges daily file counters and can supersede
  another open group sharing a version. A "read" with those effects would break the seam; the group read
  of mopm#261 is what makes the staged state observable without it.
- Staged state expires (12 hour deadline, lazily). A staged coordinate whose group expired reads `Absent`;
  the engine's observe-then-stage then creates a fresh attempt. Abandoned open groups use one of the 5
  open-group slots of the actor until then, which is an operational note for the staged shape: validation can
  take minutes, and a failed run that does not reach `abort` (a killed job) leaves its group open. Runbook: a
  refusal for the open-group cap ends the wave `REFUSED`; the operator aborts the stale groups
  (`ucdk release abort`) and reruns.

## 7. Validate, commit, lost answers, abort and yank

### 7.1 Validate: what the publisher provides

Validation is the engine's step, between `stage` and `commit` (section 17). The publisher's part is to make
the staged set provable, and to say plainly what it cannot do.

- **Where validation reads.** mopm exposes no read of staged bytes: a staged file has no URL, `fetch` of it
  answers `None`, and the group read lists declared facts, not content. The validation therefore runs on the
  engine's own sealed set from its store, whose members the engine has already verified against the set
  digest. That is also what the project's own release flow does with `mopm publish --staged <dir>
  --no-commit`: it tests the files in the staged directory and checks them against the plan.
- **What ties the validated bytes to the staged ones.** `observe(k, set)` returns `Staged(held)` only when the
  group is `open`, lists exactly the declared files of `k`'s member, and every one is `uploaded`; `held`'s
  digest is the declared `sha256`, which storage verified against the signed checksum at upload (the `PUT` is
  refused if the bytes differ, and the length is exact). The engine requires that digest to equal the digest
  of the member it validated, once after the stage (its S4) and again immediately before the commit (its
  commit step 2). So the chain is: the tests ran on local bytes with digest D, the registry holds stored bytes
  with digest D, the commit publishes those, and the read-back fetches the published bytes and hashes them to
  D again.
- **What it does not prove.** It does not prove that nobody replaced a staged file between the last
  observe and the commit: mopm gives no way to read the staged object, and the only writer is the holder of the
  actor's token and of an upload URL that lives at most 300 seconds and is not issued again for a file that is
  held. The window is one request wide, and the read-back after the commit is what notices a substitution (too
  late to prevent it, in time to refuse a smoke and yank). The manual run checks that a held file is not
  offered a URL by a repeat create.
- A group in `validating` or later is not `Staged`: its package records are not readable yet, so `observe`
  says `Absent` for it, which is the lost-answer window of 7.3.

### 7.2 Commit

Common steps, in order:

1. Validate the call with no I/O: grammar and set checks (section 5), grant checks (section 9), the call's time
   budget (section 9.3), member grouping, and the group id.
2. `POST /groups/{id}/commit`, sent once, naming each member and sending no `manifest_sha256` (the registry
   computes it, mopm#261). From the moment its first byte is written, every failure (connect, timeout, 5xx,
   lost response, unreadable body) answers `Unknown`, per the seam. A 409 `group_commit_refused`, and a 400
   `invalid_request`, are followed by a read of the group status and the version record, by this table:

   | Group status read after a refused commit | Answer |
   |---|---|
   | `committed`, `visible` | `Duplicate` |
   | `validating` | poll (step 3) |
   | `open`, after a 400 `invalid_request` | `Refused(Channel)`: the registry predates the optional manifest hash, or the body is malformed; the status read is the evidence that nothing landed, and the group stays staged for a retry or an abort |
   | `expired`, `aborted`, `superseded`, and the version record does not hold the coordinate | `Permanent`, because the status read is the evidence that nothing landed and the recovery is a new `stage`, a different call |
   | the same, and the version record holds the coordinate | `Duplicate` |
   | `rejected` | `Permanent` (numbers freed, nothing landed) |
   | `failed` | `Transient` ("numbers burned"); never `Permanent`, because the burn is an effect |
   | the status read itself fails | `Unknown` |

   A refusal that says a member is unreserved, or a 400 `upload_invalid` for a missing file, is recoverable by
   a new `stage` of the same set (the same group resumes) and gives `Transient`.
3. Poll `GET /groups/{id}` (token) every configured interval until `visible` or the call's deadline.
   `rejected` gives `Permanent`. `failed` gives `Transient` with a message that the numbers are burned.
   The deadline, a 401 on a poll (token expired) or a read failure gives `Unknown`.
4. `Committed` is returned only at `visible`: the package API (`no-store`, consistent reads) shows every member
   and the repodata and index hops have been released. That is not edge-consistency: repodata and index
   objects are `public, max-age=60`, so a reader behind the edge cache can lag `visible` by about a minute.
   API reads are exact; a smoke that reads through repodata waits for the cache to age out.

All-or-none: the group is one unit. `rejected` before the copy phase publishes nothing and leaves the
coordinates `Absent`. A `failed` group, after the copy phase, burns the numbers: channel files may sit at
exact URLs, and every member is a tombstoned record. That is **not** `Absent`, and it is not `Permanent`
either, because the burn is an effect that landed. The answer is `Transient`, the engine observes, `observe`
reads `Burned` (section 8), and the action is `REFUSED` with `PUBLISH_BURNED`: a new version is needed. A
later `stage` of that coordinate by another set answers `Duplicate` (`version_exists`), and the same
observe reads `Burned`. Visibility across subdirectories and index files is per read path, not
simultaneous; `Committed` waits for `visible`, and `observe` reads the package record, which flips at
`committed`. A reader of repodata can lag `observe`, and can lag `visible`. That is why the smoke action polls.

The race with a coordinate appearing between observe and stage: create's `version_exists` is the
compare-and-set, the answer is `Duplicate` and nothing is created. After `stage` the reservation makes the
race impossible: the numbers are held by our open group until it commits, aborts or expires. A reservation
lost in between (the group expired, or was aborted by someone holding the actor's token) shows at the commit as
the dead-group rows of the table, never as a silent overwrite.

### 7.3 Lost answers

`commit` answering `Unknown` or `Transient` is resolved by `observe`, and the group can then be in three
places. `visible`: `observe` shows `Present` with the registry's digest, and a retried commit answers
`Duplicate`. `committed` or `validating`: after `POST /commit` is accepted the group is `validating` and
package reads still show `Absent` until `commitGroup` sets `committed_at`; a retried commit sends the identical
body, which the registry treats as the same commit (202 while `validating`), and the publisher polls, so the
retry answers `Committed` or `Duplicate`, and the engine's observe and read-back tell whether the bytes are
ours. Never started: the group is still `open`, `observe` shows `Staged`, and the retry sends the commit. In
every case the publisher does not decide whose bytes they are. A test holds the `validating` window with
`observe` giving `Absent`, and a second test applies, drops the answer, lets the group reach `visible`,
retries and expects `Duplicate` and then `Present`.

`stage` answering `Unknown` is resolved the same way: the engine observes with the declared set, and the group
read says `Absent` (no group), `Staged` (every file uploaded) or neither complete, in which case the engine
stages again and the identical create resumes the same group.

### 7.4 Abort

`abort(g, ks)` computes the group id and sends `DELETE`. The engine calls it when validation fails (section
17) and from `ucdk release abort`:

| mopm answer | Publisher answer |
|---|---|
| 200 or already `aborted` | `Ok` |
| 404 `group_not_found` | `Ok` only if the group read agrees the group is gone; otherwise `Unknown` (a non-creator and an uncovered token also get 404) |
| 409 `group_not_open`, then `GET`: `expired`, `rejected`, `superseded`, `aborted` | `Ok` |
| `committed`, `visible` | `Ok`, touches nothing (only yank can undo) |
| `validating` | `InFlight`: a validating group cannot be aborted and proceeds to publish; a later observe settles it |
| `failed` | `Ok` (nothing to give back; numbers are burned) |
| 401, 403 | `GrantRefused` |

### 7.5 Yank

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
  must be 1 to 280 UTF-16 code units (mopm counts with JavaScript `string.length`, so a character outside
  the basic plane counts twice), else `Refused(Reason::Channel)` (a grammar failure).
- A pending or tombstoned version (409 `invalid_transition`) and a missing one (404 `version_not_found`) are
  `Permanent` (nothing happened, the same call fails again). A transport failure after the request was
  written is `Unknown`.
- Bytes stay fetchable: the channel specification says so, and the version record keeps the `url`; the manual
  run proves it for the real channel host.

A yanked coordinate can never be committed again: the version row persists, so create answers
`version_exists`, which maps to `Duplicate`, and the engine's observe reads `Yanked`, refused as
`PUBLISH_YANKED`.

Yank by group (`PUT /groups/{id}/yank`) is not used. Since mopm#261 the version record names its group
(`group_id`), which lets the publisher say whether a version is its own; yank by group stays unused, because
the unit is the same.

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
  coverage where the verb names coordinates) and is otherwise used only for the group read.
- **Version states.** `published` is `Present`. `yanked` is `Yanked(Held{sha256})`. A `tombstoned` record is
  never `Absent` and never `Present`. One with an empty file list is a number burned by a failed group:
  `Burned`, a presence of its own since ucdk#42 (it holds no bytes, so there is no digest to compare, and the
  engine refuses it as `PUBLISH_BURNED`, never retried). One that kept a file `sha256` (a takedown) is
  `Yanked(Held{sha256})`, which the engine refuses as `PUBLISH_YANKED`. By the source a burned member holds
   no files and a takedown keeps its digests (section 3), so the two never meet and the file list tells them apart.
   `fetch` of a tombstoned version is `None` (the record has no url). `newest` follows section 4. A
  200 whose body lacks `sha256` for a published or yanked file is `Permanent` ("malformed"); the registry's
  own response schema requires it, so only a broken emulator or gateway produces it. The publisher never
  reports `Yanked` with an empty digest to stand for a burn: an empty digest in a `Held` already means "no
  digest exposed", and a yanked build with no stamp makes a channel's supersede decision indeterminate.
- **Declared set.** `observe(g, k, set)` with a non-empty `set` computes the group id from the actor and the
  set, reads the group, and reads the package record, and combines them: package record found wins; otherwise a
  group that is `open`, lists exactly the declared files of `k`'s member (the same subdirectory, file name,
  digest and size as `set` declares), and has every one `uploaded` gives `Staged(held)`, the digest being the
  declared one that storage verified at upload; otherwise `Absent`. A group read that lists files other than
  the declared ones is a broken response (`Permanent`, "malformed"), because the id is a function of the set.
  A group that is `validating` or later is not `Staged` (7.1). Both reads use the same grant and no other call
  is made. A non-empty `set` lacking `k` is `Refused(Reason::Channel)`.
- **Empty set.** Only the package read is made. Staged state is never reported without a set, because the
  group is addressed by the declared set.
- **Another group's coordinate.** Any version published by any group or single publish reads `Present` with
  the digest mopm holds, which differs from ours when the bytes differ. It is never `Absent`.
- **Failed reads.** A failed group read or package read fails the whole observe (`Transient`, or `Unknown` if
  the answer was lost). It is never read as "no group" or "not committed"; only a 404 `group_not_found` is "no group".
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

Which envelope authorises which verb:

| Phase | Identity presented | Allows |
|---|---|---|
| Stage | publish-scope token, exact pairs | group read, create, upload, abort |
| Commit | publish-scope token (a fresh exchange) | commit, poll, yank, group read |
| Read | `Grant::none()` always | package reads only |

A test checks that a Commit envelope refuses create and upload, and that a Stage envelope refuses commit.

The grant secret that the mopm grant source produces and the publisher consumes is a length-framed envelope
in the same crate (module `grants`), so the two cannot drift:

```
version | phase | exceptional | registry location | actor | length-framed list of the five coordinate fields | token
```

The actor (the account or configured identity, an input of the group id) is carried for the group id; mopm#261
documents its forms (section 3), and where the grant source learns it is question 6 in section 16. The
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

The group id is a function of the actor and the set, and a create of a different set that shares a member
supersedes the actor's own open group unless it says `on_overlap: refuse`, which the publisher always does. With an account-acting form, every job of that account is
one actor, so a pull-request run, a re-run, or an exceptional run can supersede or reopen the release run's
staged group. `acts_via` also lets any token of the account see every group of the account, narrowed only by
package. Therefore: stage runs only from the protected release identity; exceptional and normal runs of a
machine do not share members; the envelope carries `exceptional` so a log shows it. A superseded or expired
staged group is an expected failure. A commit answers `Duplicate` only when the version record holds the
coordinate; otherwise, on evidence that nothing landed, it answers `Permanent`, by section 7's table, so the
engine observes and stages again.

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

Stage, commit and abort are three seam calls. The seam has no `Pause` or `Heartbeat` inside a call, and the
engine renews its leases immediately before a `stage`, `commit` or `abort` call and before each grant
exchange, never during one. Each call runs under a grant exchanged for its phase, so the clocks bound **one
call at a time**, not the whole publish, and the call must not outlive either:

- the **token**: 10 minutes from its exchange (`MOPM_TOKEN_TTL`, a constant of the crate with a transcript
  test of the exchange response), and the grant's own `expires_ms`;
- the **lease**: the engine's lease guard TTL (600 seconds by default) less the ledger's clock-skew allowance
  (5 seconds). A holder whose lease lapses mid-call is stale: its next fenced ledger write fails, and
  another holder may already have taken over.

The time between the calls (a validation that runs for minutes) spends neither clock: the engine exchanges
the commit grant after validation, and a fresh stage-phase grant for an abort. That is the gain of the
staged shape over a single long call: the upload gets the whole bound, and so does the wait for visibility.

The budgets, from `Config`:

| Call | Budget |
|---|---|
| `stage` | `upload_budget`: the group read, the create and every `PUT` with its retries |
| `commit` | `commit_request_timeout + poll_deadline`: the request and the in-call wait for visibility |
| `abort` | `abort_timeout`: the `DELETE` and the status read after it |

**Construction-time check:** `MopmPublisher::new` takes `Config.lease_ttl` (required, no default, set by the
caller from the TTL the engine's lease guard uses, because a default here could silently differ from the
engine's) and fails with a configuration error, before anything is sent, unless for each of the three calls

```
budget(call) + margin <= min(MOPM_TOKEN_TTL, lease_ttl - lease_skew)
```

where `margin` covers the exchange-to-call gap and one connect timeout. The error names the call that is over.
A test builds a configuration one second over the bound for each call and expects the error for that call, a
configuration at the bound and expects success, and a configuration whose three budgets together exceed the
bound while each fits and expects success (a check that summed them would refuse it). Proposed defaults: upload
300 s, commit request 30 s, poll 420 s, abort 15 s, margin 60 s: 360 s for a stage, 510 s for a commit and 75
s for an abort, against a bound of 595 s. The poll is long on purpose: the engine's read-back polls only
`Publish.polls` times (default 3, one tick apart), so a wait for visibility that ends early becomes
`INDETERMINATE`.

Per call, with the injected clock: at entry the remaining life of the grant must cover the call's budget, else
`GrantRefused(Expired)` with nothing sent; the call computes one deadline from the entry time and checks it
before every request. A `stage` out of budget stops between files: the group stays open and the next `stage`
resumes it (`Transient`). A `commit` out of budget before `POST /commit` is written stops with `Transient`; out
of budget after it is written, or a 401 on a poll, is `Unknown`. The engine's observe and read-back settle it.
`poll_deadline` may be set to zero, which returns `Unknown` as soon as the commit is accepted, but then only
the engine's read-back waits. The default stays above zero and the design asks nothing of the engine here.

**Stale-holder outcome.** If the lease lapses despite the bound (a stalled host, a clock step), the stale
call's commit may still land. That is safe, not harmless-by-luck: the group id is a function of actor and set,
so the successor's observe finds the same group (`Staged`, or `Absent` while it validates) and its commit is
the identical request, which the registry treats as the same commit; the successor's write-ahead intent is
already in the ledger; and the stale engine's next ledger write is refused by the fence and it stops. The
publisher returns `Unknown` in that window, never `Permanent`.

**Re-exchange (M2).** The maintainers agreed that a client may, on a 401 `token_expired`, request a fresh
identity token and exchange again (the old `jti` is single use; a fresh token works). That is a grant-source
act, and a publisher receives a `Grant` by value with no way to ask for another. v1 therefore does not rely
on it and the budget check above is the guard. Using it inside a call would need a seam addition (a renewer
handed to the publisher at construction) and is a decision for the seam owners (question 3 in section 16).

## 14. Decisions

1. **The staged shape is the shape: stage, validate, publish, with abort. `stages()` is `true`, and there is
   no one-call mode.** Why: it is the pattern every package manager has, mopm can stage (`--no-commit` opens
   an open, invisible group), the project's own release flow already works this way, and it gives the engine
   the one step that matters, tests on the exact set before anything is public. Alternatives: a one-call
   shape first (one long call that outlives a lease more easily, no step to validate in, and a second path to
   keep, with the staged shape added behind it later); keeping a one-call fallback (nothing needs it once
   mopm stages; a channel that cannot stage is another channel's special case, section 17); a local record of
   group ids (a second source of truth, lost with the machine, and it fails the lost-answer scenarios).
2. **One crate, `ucdk-release-mopm`, containing the publisher, the grant envelope codec and the mopm grant
   source (PR 4).** Why: the envelope is a private contract between its halves. Alternative: a separate
   grant-source crate (two crates must agree on an undocumented format).
3. **`CommitUnit::Set`, `conditional_put() == true`.** Why: group commit is all-or-none and create is
   compare-and-set. Alternative `Coordinate` would break atomicity across platforms of one version.
4. **`observe` reads the package record (and, for a declared set, the group), never `list` or search; the
   read-back fetches and the engine hashes.** Why: seam rule; the record is the authoritative per-version
   read, and a registry's own digest proves only what the registry says.
5. **A version sealed with another file set is `Refused(Channel)` on write and `Absent` on observe, never
   `Duplicate`, except a burned (tombstoned) record, which is `Duplicate` and reads `Burned`; a
   multi-platform release must publish all platforms in one set with one manifest (section 5.1).** Why: avoids
   an observe-`Absent`, stage-`Duplicate` loop; mopm will not change. Alternative: widen `Present` to the
   whole version (lies about a coordinate not held).
6. **`Committed` is returned at `visible`.** Why: API reads are exact then and every repodata and index hop has
   been released; edge caches still lag by up to 60 seconds, which the engine's read-back and a smoke must
   tolerate. Alternative: return at `committed` (shorter call, but the hops are not released).
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
11. **No manifest hash is computed or sent.** Why: `manifest_sha256` is optional on commit (mopm#261) and the
    registry computes it, so the client has no second path to keep. The archive is still read before any
    I/O, to refuse early what the registry would reject (a group spends the daily allowance per attempt):
    the facts of section 5, and agreement of the files of one member.
12. **`grant_limit()` is 100 coordinates; a set with two versions of one name is refused.** Why: a bound in
    coordinates is never over mopm's distinct-name limit. Alternative: count distinct names (a trait and
    two kits change).
13. **A PRIVATE or UNSPECIFIED repository is refused at construction, an anonymous push identity cannot
    write, and the grant source guards on `repository_visibility`.** Why: mopm cannot honour private reads
    and an anonymous write has no token.
14. **No environment variables.** The registry, timeouts, caps and trust bundle are configuration; the
    credential is the grant.
15. **A `group_conflict` is classified by `error.reason` alone; no message is parsed; an absent or unknown
    reason is `Unknown`.** Why: a wording change must not change a class, and a wrong `Duplicate` loops the
    engine. A reason is a closed, documented list (mopm#260), so it is a contract.
16. **A 404 is `Absent` only with a parsed `package_not_found` or `version_not_found` body (and "no group"
    only with `group_not_found` on the group read).** Why: a mis-pointed registry must not look empty.
17. **A tombstoned record is never `Absent`: `Burned` without a digest, `Yanked` with one.** Why: a burned
    number can never be committed again, and a terminal state must be a refusal, not a retry.
18. **A failed group answers `Transient`, never `Permanent`.** Why: the burn is an effect; the engine's
    observe reads `Burned` and refuses. `Permanent` is allowed only where a status read proves nothing landed.
19. **Each call is bounded below the lease and the token it runs under, checked per call at construction
    (section 9.3).** Why: the seam has no heartbeat inside a call and the engine renews only before it; the
    calls are separate, so the bound is per call and the time of a validation costs neither clock.
20. **`on_overlap: refuse` is always sent; a registry that does not honour it is a requirement proven by the
    manual run, and a `superseded` list under `refuse` is refused as a contradiction (section 6).** Why: the
    create answer has no positive marker of honour, so a runtime check would be a claim with nothing behind it.
21. **Validation reads the engine's local sealed set, tied to the staged one by the digest `observe` reports,
    because mopm exposes no read of staged bytes (section 7.1).** Why: it is the only evidence available and
    the read-back closes the chain. Alternative: ask mopm for a read of staged bytes (a new surface, a new
    way to leak unreleased bytes, for a check the digest already makes).

## 15. PR plan

Each PR is small, passes `cargo xtask check` and the 99% line and branch coverage bar for the crates it
touches, and ships tests that failed first for any fix.

1. **Skeleton and mapping types.** Crate manifest (not `pure`), licence headers, `Config` (no default
   registry, `Roots::Bundle` only, `lease_ttl` and the construction-time budget check, per call: stage,
   commit, abort), `Transport` trait and types, `Secret` types, coordinate-to-member mapping with mopm's
   exact grammars and the platform table, set checks, error mapping table (including the closed list of
   `group_conflict` reasons and their classes), the `ureq` dependency checked against `deny.toml`. Tests:
   mapping and injectivity (coordinates differing only where a `/` falls), the platform table both ways,
   grammar refusal for observe too, an invalid-grammar version never produces a request, every row of the
   error table, each call's budget one second over the bound and at it and the three together over it, a
   bogus certificate variable changing nothing, the version length, empty file, 20 GB and per-member manifest
   agreement refusals, the archive's subdirectory against the table, the `Retry-After` ceiling.
2. **Stage and abort.** The group id (computed from the actor and the set, pinned by the vectors of
   mopm#261), the group read, `stage` (read, create with `on_overlap: refuse`, the `reserving` repeat,
   uploads of the files not held, the id check, every `group_conflict` reason's class, the `superseded`
   contradiction) and `abort` (the table of 7.4). Starts when mopm#261 has merged, so the wire contract is
   frozen by its tests. Tests on a hand-built fake `Transport`: a repeat of the same set resumes and uploads
   nothing held, a different set sharing a member is refused and ends nothing, a lost create answer, an
   expired upload URL, every table row.
3. **`observe`, `newest`, `list`.** Package read, the group read for a declared set (`Staged` only for an
   open group listing exactly the declared files, all uploaded), 404 kinds, tombstone mapping (`Burned`,
   `Yanked`), the sparse-index read with its fallback, ambiguity rule, caps. Tests for lag, failed reads never
   `Absent` (a bare 404 first), a group read that disagrees with the declared set, a failed read of the third
   of five versions, no digest, a burned member skipped.
4. **Grants.** Envelope codec, repository, phase, coverage, expiry checks on every verb with an empty request
   log on refusal; the 401 mappings; `MopmGrantSource` (one identity token per phase, `Used` and
   `Unreachable`, the grammar check before the exchange, the visibility guard).
5. **`commit`, `fetch`, `yank`.** Commit with no manifest hash, the poll, the status table of 7.2, `Unknown`
   after send; the file read; the yank refusals. Tests on a fake `Transport`, including apply-then-drop, the
   `validating` window with `observe` giving `Absent`, a failed group answering `Transient`, a dead group with
   and without the coordinate held, a 400 on commit with the group still open, and the yank refusals (not
   held, uncovered sibling).
6. **Emulator and rig; kit.** `ucdk-release-mopm-emulator` (section 11), `MopmRig` with the hooks, and
   `run_publisher_kit` for the staged shape, plus the TCP-stub adapter tests. Lands after kit parts 6 and 7
   (4g, 4h).
7. **Manual non-production conformance job** script and its documentation: protected environment, manual
   dispatch, the non-production marker check, the overlap probe and the checklist of section 11. Scenarios are
   marked proven against real mopm only after this run. Needs mopm#261 deployed.

Outside this crate and in this order of need: the engine and machine change of section 5.1 (before any
multi-platform release); the validate phase of section 17 (before the first release that has tests to run on
the staged set); the maintainers' M1 (mopm#261), then M6 and M8 (in the same change), then M10, then M3
(section 13); the yank verb's sibling rule in the engine.

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

**What revision 5 changed.** On the maintainers' direction the staged shape is primary: the summary, goals,
mapping, sections 6 to 9, the scenario table, the decisions and the PR plan are written for stage, validate,
publish, with abort, and the one-call shape is dropped (decision 1). mopm#260 is live: `group_conflict` is
classified by `error.reason` (section 6), `on_overlap` is always `refuse`, and the honoured check is reduced to
what an answer can show (decision 20, M16). mopm#261 is in review: the group id is computed and the group read
makes the staged state observable (sections 3, 4, 8), the manifest hash is neither computed nor sent (decision
11), and `getVersion` names the group. The lease and token bound is per call (9.3). Section 7.1 says where
validation reads its bytes and what that proves, and section 17 lists what this changes in the seam (a comment)
and in the engine documents (the validate phase, the abort after a failed validation, where `committing` is
written). What earlier revisions established stays: the error classes, lease and token time, grants,
the read-back hashed by the engine, the multi-platform declared set, burned and yanked handling.

**Open questions**, each with a recommendation:

1. **Multi-platform mechanism (engine owners).** (1) a declared platform set, (2) one joined Publish set, or
   (3) refuse at admission a split publish for a sealing channel. Recommend (3) now and (1) with (2) next.
2. **The stamp has no home on mopm.** The kit's base `Held` carries a stamp (build number and source commit)
   and an exceptional mark, and the list, newest and stamp scenarios compare the whole `Held`. The
   maintainers took a typed `source_commit` (M3) only. Recommend asking for `build_number` and `exceptional`
   as typed fields too, and moving M3 ahead of M10; the alternative is a seam declaration that a channel
   stores no stamp, with the engine refusing a monotonic rule on it.
3. **Re-exchange inside a call** needs a renewer handed to the publisher (a seam addition). Recommend not
   adding it in v1; the per-call budget check suffices.
4. **Burned handling after a `Transient` stage, and the read-back bound (engine owners).** `release.md` states
   the observe that reads `Burned` after a `Duplicate` and in the commit steps, not after a `Transient` in the
   stage phase, which the `group_failed` row relies on. And the read-back bound (3 polls) is shorter than a
   minutes-long validation if `poll_deadline` is zero. Recommend stating the first and leaving the second as
   the default above zero in 9.3.
5. **A commit on a dead group whose version is not held answers `Permanent`**, on the evidence of the status
   read (the kit's lost-staged-state scenario requires an error that promises no effect). The review asked for
   `Transient`; the difference is whether the engine retries the same commit or restages. Recommend
   `Permanent`, and say so in the runbook.
6. **The actor string** is an input of the group id and mopm#261 documents its forms (`u:<GitHub id>`,
   `ci:gh-repo:<repository id>`). The grant source must produce it from the identity it exchanged; recommend
   the manual run compares the id the create answer names with the computed one (6), and that the exchange
   response carries the actor if the maintainers will say so.
7. **Stored digest for the preflight decision (seam owners).** `observe` reports the record's `sha256`, which
   storage verified at upload, without a fetch (the seam text allows it). `decide_publish` then separates
   "same bytes" from "other bytes" on a digest nothing re-checks after the copy. Recommend accepting that for
   the preflight and commit phases, because the read-back fetches and hashes before any success.
8. **The validate phase (engine owners).** Which validation kind a Publish action may hold that runs on the
   staged set, and its place in `machine.validation_placement`. Recommend one kind that reads only the engine's
   local set (never the channel), so the same kind serves a staging channel (run after S4) and a non-staging one
   (run before the commit), and that the project's tests of the set are what it runs.
9. **The abort after a failed validation (engine owners).** Recommend the engine exchanges a fresh stage-phase
   grant for it (the stage grant is dropped at S4 because a validation may outlast its life), counts it outside
   `Publish.attempts`, and reports the abort's `InFlight` or failure as staged state left for
   `ucdk release abort`.
10. **`stages()` wording (seam owners).** Recommend rewording its comment to name staging as the primary shape
    and `false` as the declared capability, with no change of signature and no default (section 17).
11. **A marker of `on_overlap` honoured (maintainers).** Recommend asking for the create answer to echo the
    value it applied (M16, section 13); until then the manual run's probe is the only proof.

## 17. What this changes in the seam

The maintainers' direction is that stage, validate, publish, with abort, is the shape the seam and the engine
express first, and that a channel that cannot stage is the special case. This section says what that needs, by
reading `crates/ucdk-release-seams/src/publisher.rs` and `grant.rs` on `main` and sections 4.3 to 5.3 of
[release.md](release.md).

**The seam: one comment, nothing else.**

- The trait already models the shape. `stages()`, `stage`, `commit` and `abort` exist; `Presence::Staged(Held)`
  is "held by the registry and not readable by a stranger"; `StageAnswer::Staged | Duplicate`; and
  `observe(g, k, set)` takes the declared set precisely so that a registry that addresses staged state by a
  group derived from the whole set (mopm) can answer "staged with our digest". No signature changes, and no new
  verb is needed for validation: it is the engine's step and reads the engine's own set.
- **`stages()`: reword its doc comment, keep it required.** Today it reads "True if the channel holds staged
  bytes that are not yet readable. False: `stage` and `abort` are never called and `commit` carries the bytes."
  That is accurate; the change is to say that staging is the primary shape and `false` is the declared
  capability of a channel that cannot stage. It stays a required method with no default: a default of `true`
  would let a channel that cannot stage inherit a claim it cannot keep, and a default of `false` would make
  the special case the default, which is what the direction reverses.
- **No read of staged bytes is added.** A registry that exposed one could let validation read the channel's
  copy; mopm exposes none, and the digest tie of 7.1 (the staged digest equals the digest of the member the
  engine validated, checked at S4 and again at the commit) is what the seam already supports. `fetch` of a
  staged coordinate stays "nothing for a stranger".
- **The kit** needs no new scenario for the shape: rows 2 to 9 and 27 to 30 of section 12 are the stage,
  observe, commit and abort contract, and the mopm rig declares only `{stages: true}`. The kit's helpers that
  make a non-staging rig run the same fixtures stay for the channels that need them.

**The engine documents (`release.md`), proposed in this change:**

1. **A validate phase between the stage phase and the commit phase (5.2).** After S4 the engine runs the Publish
   action's staged-set validations on its own sealed set (verified as in the preflight), tied to the staged set
   by the digest S4 already requires. A failure ends the action `VALIDATION_FAILED` before anything is public.
   Today 5.2 says a kind that needs staged state "is not part of v1" and that a failed validation never leaves
   staged state; both change.
2. **The abort after a failed validation is the engine's, not only `ucdk release abort`'s.** It uses a fresh
   stage-phase grant, because S4 drops the stage grant (a validation may outlast its ten minutes), and each
   intent is removed once an observe shows `Absent` (5.2, 5.3).
3. **`committing` is written after validation, not at S4.** The intent's `phase=committing` means "a commit may
   have started", so it is rewritten when the validate phase has passed, immediately before the commit phase.
4. **A non-staging channel runs the same validations on the local set before its commit**, with no abort, so
   the validation step does not depend on the channel's capability.
5. **4.3 and PR 14 (11):** the first real publisher has `stages() == true`; the revision-4 text that made it
   `false` first is reverted.
6. **The ambiguity table (5.3)** gains the failed validation, and the interrupted run re-checks the digest
   tie by its observe and skips a validation already `Confirmed` for the same set (5.4).

Not decided here, and asked of the engine owners (section 16, questions 8 and 9): the validation kind and its
placement rule, and the abort's grant and attempt accounting. The seam is unaffected by either.
