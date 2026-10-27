# The mopm release Publisher: the rig, the scenarios and the changes asked of mopm

Status: design, revision 2. This is the second half of [release-publisher-mopm.md](release-publisher-mopm.md),
split out to keep both files under the project's size bound. It holds sections 11 to 13; section numbers
are global, and sections 1 to 10 and 14 to 16 are in the first file. Nothing here is implemented yet.

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
    (and, once M14 is modelled, `on_overlap: refuse`), the open-group and supersede caps, the `failed`
    group's `group_conflict`, and a `group_conflict` reason field when M13 is modelled;
  - 404 kinds, tombstoned version records, signed-checksum PUTs, yank of a whole version, token checks, the
    sparse index (with lag behind the package record), a 429 with `Retry-After`, and the `reserving` answer.
- **The kit's rig hooks.** The seam owners decided one `PublisherRig` shape, once, in the first publisher kit
  part (4g), with defaults that reproduce today's behaviour so the fake's rig does not change. The mopm rig
  implements:
  - `shapes() -> Vec<ChannelShape>`: required. mopm declares `{stages: false, private: false}` and, once the
    staged shape exists, `{stages: true, private: false, group_addressed: true}`. It never declares
    `private`. The kit asserts `built.stages() == shape.stages` for every publisher it builds, holds a
    `private: false` rig to `public_read_needs_no_grant`, and fails a rig whose `shapes()` is empty. A
    skipped scenario is reported by name with its reason, and is never counted as a pass.
  - `coordinate(label) -> Coordinate`: maps the kit's label to a coordinate mopm can hold: a name of two or
    more characters matching the grammar (the label with a fixed suffix, so a label that is a prefix of
    another stays a prefix, which the listing scenario needs, and distinct labels stay distinct; the kit
    checks both once), the configured channel, type `CONDA` (what `types()` returns), version `1.2.3`, and
    the ucdk platform `linux-x86_64` (the publisher maps it to `linux-64`, section 5). The kit's literal
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
    the grammar, a prerelease version, an unknown platform, and two coordinates differing only where a `/`
    falls. Every verb on them answers `Refused` and `observe` answers `Refused`, not `Absent`.
  - `seals_versions() -> bool`: `true`. Checked both ways: a rig that says `true` must pass
    `a_sealed_version_refuses_a_later_platform_and_observes_it_absent`, and one that says `false` must
    commit a sibling beside a held coordinate. This is the kit's form of section 5.1.
  - `can_burn() -> bool`: `true` for the emulator, which turns a group `failed` and burns its members.
    Real mopm offers no way to arrange a failed group on demand, so the burned scenario runs on the
    emulator only; the note is in the kit's PR.
- **Helpers ask the shape.** The kit's `stage()` helper does nothing and `publish()` is a `commit()` when
  `p.stages()` is false, so a one-call rig runs the ordinary fixtures (most scenarios use a staging shape
  only as a default) and not just the handful that name a one-call shape. A lost non-staging commit may
  observe `Absent` (before) or `Present` (after); a retry may answer `Committed` or `Duplicate` and must be
  followed by an `observe` that is `Present` with our digest.
- **`MopmRig: PublisherRig`** in the publisher's `tests/` directory, built on the emulator.
  - `publisher(shape)` builds the publisher in the one-call or staged shape, or declines.
  - `arrange` maps to emulator faults: `HeldByAnother` (another actor publishes the version),
    `StagedByAnother` (another actor's open group holds it), `RaceNextCommit` (the version is taken after
    our reads and before our create; in the staged shape the rig kills our reservation and a rival commits,
    the answer being `Duplicate`), `LoseStagedState` (the reservation is lost and nobody holds the version:
    an error that promises no effect, `observe` `Absent`, a second stage and commit works),
    `CommitVisibleAfter(n)` (the validation delay), `BurnedByFailedGroup`, `LoseAnswerAfterCommit` (apply,
    then drop the response, with the validation delay set), `LoseAnswerBeforeCommit` (drop the request),
    `FailAfterCommit` (apply, answer 503), `CommitSetFailsWhole` (validation rejects, and a second variant
    where the group `failed` and burns), `ListLags` (search omits a version), `FailGroupRead`,
    `FailIndexRead`.
  - `grant` and `foreign_grant` build envelopes; `advance` moves the injected clock; `reads` returns the
    request log mapped to Group and Index steps.
  - **Rig step 1 provisions the identity form under test.** For the configured form there is no claim: the
    first exchange creates the package, and the rig asserts that. For the `acts_for` form the rig claims
    every package of the scenario for the account first. The emulator does both in process; the manual run
    does the first against a beta deployment and the second only with an operator-supplied claim.
- **Fidelity.** An emulator can drift from mopm. Three guards: (1) emulator responses are validated against
  mopm's published API description vendored into the emulator crate with its digest; (2) one
  recorded-transcript test per mopm error code the mapping table uses, and one per `group_conflict` reason
  class once M13 exists; (3) a manually run job (not in CI, not in any pull request) executes the same kit
  script against a non-production mopm deployment with an identity provisioned for that purpose. It starts
  only from a protected environment by manual dispatch, never from a fork pull request, and checks before
  any exchange that the registry's index configuration names a non-production stage (the index's `stage`
  value is already the marker; the guard is ours, M5), refusing with nothing sent if it does not. `Config`
  has no default registry. No production registry is ever a rig target. The manual run's checklist records:
  that group create accepts a package the configured exchange created unclaimed; the uncached strongly
  consistent version record; `Present` at `committed` with repodata lagging; all members of a 20-member
  group reading `Present` in the same observe round; that `files[].sha256` equals the SHA-256 of the
  downloaded bytes; whether a burned member keeps a digest; whether the configured exchange applies the
  reserved-name check; the group id derivation against the documented one (M1); yank retention at the
  channel host; and the real token exchange and its 10 minute life (section 9.3).
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

OC is the one-call shape, ST the staged shape. Every ST cell presupposes the staged shape, which exists only
after M1, M13 and M14 land, so those three are not repeated per row. "Emulator" marks a row whose evidence
is only the in-process emulator even after the manual run.

| # | Scenario | OC | ST |
|---|---|---|---|
| 1 | observe_absent_is_not_an_error | met: `Absent` only for a parsed 404 `package_not_found` or `version_not_found`; a missing platform of a held version is `Absent`; the kit's sibling checks use `siblings()` (platform `SameSet`, version `OwnSet`) | met |
| 2 | stage_then_observe_returns_staged_with_the_same_digest | skipped | met: create and PUT, then group read plus package read; the declared digest storage verified |
| 3 | a_staged_coordinate_is_not_readable_by_a_stranger | skipped | met: mopm hides pending rows from every read; `fetch` follows the record's url only |
| 4 | a_stage_grant_observes_the_staged_state_it_made | skipped | met: group read with the stage envelope |
| 5 | a_commit_grant_observes_it_too | skipped | met: same token class, commit envelope |
| 6 | commit_makes_the_staged_coordinates_readable_with_the_same_digest | met: commit to `visible`, `Present` with `files[].sha256` | met |
| 7 | stage_of_present_same_is_duplicate_and_changes_nothing | skipped | met: group read, or the `version_exists` preflight, nothing reserved; platform siblings go `SameSet` |
| 8 | commit_over_present_different_leaves_old_bytes | met: `Duplicate`, old bytes stand | met: stage gives `Duplicate` first; a commit on a dead group gives `Duplicate` when the version is held |
| 9 | abort_gives_staged_state_back_and_never_touches_a_committed_coordinate | skipped | met: abort table of section 7; `committed` gives `Ok` without effect |
| 10 | a_set_commit_makes_every_coordinate_readable_or_none | met: one group; `rejected` leaves `Absent`; `failed` leaves `Burned`, never `Absent`; emulator models both | met |
| 11 | a_channel_that_does_not_stage_commits_with_the_bytes | met: the shape itself, with archives from `payload` | skipped |
| 12 | no_digest_exposed_is_refused | met on the emulator only: real mopm exposes `sha256` for every file, so a 200 without it is `Permanent` ("malformed") and only a broken emulator produces it | same |
| 13 | yank_keeps_bytes_fetchable | GAP until kit 4g adds the `yank_unit()` branch: the draft scenario says a yank reaches only the coordinate it names, which mopm's `Version` unit cannot pass; with the branch it is met (siblings named and covered: both yanked; one omitted: `GrantRefused(Denied)`, nothing changes). Retention at the channel host is the manual run's. | same |
| 14 | yanked_coordinate_cannot_be_committed_again | met: `version_exists` gives `Duplicate`, observe reads `Yanked` | met |
| 15 | lost_answer_is_resolvable_by_observe | met: `Unknown`, then `Present`, or `Absent` (a non-staging channel may observe it), or during `validating` a retry that gives `Committed` or `Duplicate` | met: `Staged` or `Present` |
| 16 | list_may_lag_but_observe_is_exact | GAP until M3 is widened (question 2): the listing helper compares the whole `Held`, stamp and exceptional mark included; observe is exact from the uncached record | same |
| 17 | public_read_needs_no_grant | met: all reads unauthenticated; a write without a grant is `Denied` | met |
| 18 | a_grant_for_another_repository_sends_nothing | met: envelope check on every verb including reads; request log empty; proves the envelope logic only | met |
| 19 | a_grant_of_the_wrong_phase_is_denied_and_nothing_happens | met client side (proves the envelope logic only: a stage and a commit token are both mopm `publish` tokens; the channel and type siblings the draft check used are `unrepresentable()` instead); server enforcement after M10 | same |
| 20 | an_expired_grant_is_refused_and_nothing_happens | met: injected clock, checked on every verb; 401 `token_expired`; the real exchange is the manual run's | met |
| 21 | newest_lists_every_held_coordinate_of_the_name | GAP until M3 is widened: the scenario compares digest, stamp and exceptional mark of every held coordinate; the digests, the yanked ones, the empty-package case, the all-or-nothing read and the cap are met | same |
| 22 | observe_returns_the_stamp_it_was_given | GAP: M3 as taken stores `source_commit` only; the build number and the exceptional mark have no home (question 2) | GAP |
| 23 | a_granted_observe_sees_a_coordinate_committed_by_another_writer | met: public read | met |
| 24 | a_private_observe_reads_the_group_then_the_index | skipped: mopm declares no private shape (G-private) | skipped |
| 25 | a_coordinate_committed_in_another_group_reads_as_present_never_absent | met: `Present` with mopm's digest | met |
| 26 | a_failed_group_read_or_index_read_fails_the_observe_and_is_not_absent | met for the public shape: a bare 404 or a 404 with another code is `Transient`; the private half is skipped | met after M1 for the group-read half |
| 27 | observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set | skipped | met after M1: the group id recomputed from the declared set |
| 28 | observe_with_an_empty_set_cannot_see_staged_state_and_still_sees_committed_coordinates | met: package read only | met |
| 29 | a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit | met: read is `Grant::none()`; a Read envelope carrying a token is refused; a write is refused client side | met |
| 30 | commit_is_atomic_when_the_coordinate_appears_between_observe_and_commit | met: `version_exists` gives `Duplicate`, winner stands | met: realized as a lost reservation plus a rival commit, answer `Duplicate` only |
| 31 | transient_after_commit_is_observed_before_retry | met: `Unknown`, never `Permanent` | met |
| 32 | duplicate_over_other_bytes_leaves_the_old_bytes | met: `Duplicate`; platform siblings go `SameSet` | met after M13 and M14: a stage of other bytes over one's own staged coordinate is refused as an overlap, never superseded |
| 33 | a_second_commit_of_the_same_bytes_is_duplicate | met: 200 `skip_identical` on a `committed` or `visible` group is `Duplicate`; on a `validating` group the retry may give `Committed`, which the kit accepts | met: body status `committed` or `visible` at entry |
| 34 | a_stage_over_ones_own_staged_coordinate_is_duplicate | skipped | met after M13 and M14 (identical set: group read, `Duplicate`; other bytes: the overlap reason) |
| 35 | a_commit_carries_exactly_what_was_staged | skipped | met after M1 and M8 for bytes (declared digests read from the group; the server computes the manifest hash); the stamp half is a GAP with row 22 |
| 36 | a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was | met: preflight before reserve; a conflict mid-reservation aborts the group server side | met |
| 37 | a_set_that_names_a_coordinate_twice_is_refused | met: client check, no I/O | met |
| 38 | a_stage_or_commit_grant_reads_no_staged_state_but_through_observe | met: `fetch`, `newest`, `list` are public reads and carry no token | met |
| 39 | a_channel_that_does_not_stage_creates_a_coordinate_once | met: lost race, yanked recommit, post-landing failure `Unknown`, failed set commit (rejected and burned) | skipped |

Scenarios the seam owners' answers add to the kit's last two parts, and their status for mopm:

| Scenario (name as decided) | Status |
|---|---|
| a_burned_coordinate_is_neither_absent_nor_present_nor_committable | met on the emulator only (`can_burn`): `observe` `Burned`, stage and commit `Duplicate`, `fetch` `None`, `newest` omits it |
| a_sealed_version_refuses_a_later_platform_and_observes_it_absent | met (`seals_versions`): `Refused(Channel)`, `Absent` |
| a_coordinate_the_channel_cannot_hold_is_refused_before_anything_is_sent | met: every verb `Refused`, `observe` `Refused` |
| a_retry_of_a_commit_that_is_becoming_readable_is_committed_or_duplicate_and_never_a_second_write | met: one landed write, then `Present` with our digest |
| a_commit_of_staged_state_that_vanished_is_not_committed_and_a_restage_works | skipped in OC; met after M1 in ST (`Permanent` on the status read's evidence, section 7) |
| a read-back that fails a registry echoing the declared digest (4g, named there) | met: `fetch` returns the bytes served and the engine hashes them |
| the yank-unit `Version` branch | see row 13 |

Counted from the 39 rows above (a cell that says "same" takes the status of its neighbour): the one-call
shape has 25 met, 10 skipped and 4 GAP (rows 13, 16, 21, 22); the staged shape has 27 met, 5 met after a
mopm change (rows 26, 27, 32, 34, 35), 3 skipped (rows 11, 24, 39) and 4 GAP (rows 13, 16, 21, 22). "Met"
includes row 12 (emulator only) and row 19 (client side only). Row 24 is skipped in both. A skip is reported
by the kit and is not a pass, so the one-call shape passes 25 of 39 and not 35.

## 13. Changes asked of mopm, the maintainers' verdicts and the agreed order

Every verdict was checked against the mopm sources at commit d062f75. All taken changes are additive and
wire-compatible. Service changes reach a deployment through its own promotion path; this repository changes
none. "Before" says what the publisher does until the change lands, which is what ships first.

**Order requested of the maintainers.** First the client-side re-exchange (M2, no deploy) and the
documentation items (M11 and the token-form notes); then M13 and M14; then M1, M6 and M8; then M10; then M3.
Question 2 of section 16 asks to move a widened M3 ahead of M10, because three kit rows depend on it.

| Id | Verdict | What mopm does | Before it lands the publisher | After |
|---|---|---|---|---|
| M13 | take (stage 1) | keep 409 `group_conflict` and add a stable `error.reason` from a closed documented enum | maps every `group_conflict` to `Unknown`: the engine observes and the action ends `INDETERMINATE` after the attempt bound, **never `Duplicate`**; no message is parsed; an operator who hits the open-group cap aborts the stale groups and reruns | maps by reason class (section 6 table) |
| M14 | take (stage 1) | `POST /groups` takes `on_overlap: supersede` or `refuse`, default `supersede` (today's contract) | sends nothing new; the staged shape is not built, and in the one-call shape the engine's per-channel lease and the rule that exceptional runs share no members (section 9.1) keep overlap rare | sends `refuse` always; rows 32 and 34 in ST become met |
| M1 | take, modified (stage 2) | `GET /groups/{id}` returns per-member `files [{subdir, filename, sha256, size, uploaded}]` (`uploaded` only while open), and the group id derivation (actor plus declared set) becomes a documented, test-pinned interface; the publisher recomputes the id and reads it (404 is no group). Never create to inspect: create on an ended group reopens it and charges the allowance | builds the one-call shape only; no staged shape, no local record of group ids | builds the staged shape; `observe` with a declared set, `stage`, `abort` use the group read |
| M6 | take (stage 2) | `getVersion` emits `group_id` when the version came from a group | decides whose bytes a version holds by digest alone (the seam's rule); the version record cannot name its group | may tell its own group from a foreign one, and tell a burn from a takedown by the group's status |
| M8 | take (stage 2) | `manifest_sha256` optional on commit: present is today's check, absent is server computed | extracts it client side from the archive in one small module; an invalid archive is `Permanent` before any I/O | stops sending it and **deletes the module**; an invalid archive is then a `rejected` group, not a pre-check, so no second path survives |
| M10 | take, raised to stage 3 | a new token scope `stage` (create, upload, get, abort); commit and yank require `publish` and `yank` | phase discipline is client side only (section 9) | the grant source asks for the scope of the phase; the server enforces the table |
| M3 | take, modified (stage 4) | one typed immutable field `source_commit` (a git sha) on commit, returned by `getVersion`; no free-form map | stores no stamp: `observe` reports `stamp: None` (truthful, and the engine already reads it as unstamped, which makes a supersede decision indeterminate); refuses an item whose `Held` carries the exceptional mark (`Refused(Channel)`, nothing sent) rather than record it as ordinary | stores and returns the source commit. The build number and the exceptional mark still have no home: rows 16, 21, 22 stay GAP (question 2) |
| M2 | take (no deploy) | client side only: on a 401 `token_expired` re-request the identity JWT and exchange again; no longer token, no refresh route | does not rely on it; the call is bounded below the token and lease (section 9.3) | the grant source may re-exchange; using it inside a call needs a seam addition (question 3) |
| M9 | take | group scenarios and goldens in the existing wire harness, not a separate transcript system | hand-written transcripts plus the manual run | goldens generate guard 2 of section 11 |
| M5 | take as docs (no config change) | none: the index config's `stage` value is already the non-production marker | the manual run reads it and refuses a registry whose stage is not non-production (ours to enforce) | same |
| M11 | docs only | state that `getVersion` is `no-store`, and the token forms and `ci_owner` of group create | proven by the manual run at the channel host | the API description says it |
| M7 | already exists | one anonymous GET of the sparse index gives every file digest | `newest` uses it (section 4), falling back to the record for a version the index lags on | same |
| M15 | already exists | CI publishers are configured per deployment; the publishing identity needs a row in the deployment's list, which the operators add on request | the identity must be requested before the manual run; rig step 1 documents it | same |
| M4 | rejected | none; the cost of a conflict mid-reservation is documented instead: a number taken after the preflight aborts the group and the next identical create is charged against the daily file allowance | states that cost (section 3) | same |
| M12 | rejected | none; on `version_exists` the public `getVersion` `files[]` set is the comparison | compares that set (section 5.1) | same |
| GAP-S | rejected | none; the publisher decides the full platform set before the first group | refuses a later platform (`Refused(Channel)`), and a multi-platform release needs the engine and machine change of section 5.1 | same |
| G-private | out of scope for mopm | none; the guard is ours | refuses a PRIVATE repository at construction and guards on the identity token's `repository_visibility` claim (section 8) | same |
