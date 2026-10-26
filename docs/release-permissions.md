# ucdk release: permissions

Status: design (Proposal). This page is the full text of section 6 of [release.md](release.md). It says how a release machine gets permission to stage and to commit a set in a channel, what it records about each grant, and what happens when a grant is refused, expired or already used. Section numbers below without a file name are those of release.md.

## 1. What needs a grant

Only an action that writes to, or reads a private part of, a channel needs a grant. A build, a test, a validation and a read of a `PUBLIC` repository need none.

| Effect | Grant | Why |
|---|---|---|
| Publish action, stage phase: preflight reads of a `PRIVATE` repository, `stage` | the **stage grant**, scope `Federated.stage_scope` | puts bytes in the channel where nothing can read them |
| Publish action, validation phase: `abort` of the staged state after a failed validation | a stage grant (exchanged again if the first is no longer good) | it undoes what the stage phase did and makes nothing readable |
| Publish action, commit phase: reads of a `PRIVATE` repository, `commit`, read-back | the **commit grant**, scope `Federated.commit_scope` | makes the set readable and permanent |
| Smoke action on a `PRIVATE` repository | a commit grant | it reads what the commit made |
| `ucdk release abort` | a stage grant | same as the validation-phase abort |
| `ucdk release yank` | a commit grant for the one coordinate | it changes what is published |
| A channel that does not stage (`Publisher::stages()` is false) | the commit grant only, covering the write | there is no stage phase |
| `Anonymous` push identity | none | the channel needs no authority to write |

**The two grants are separate on purpose.** The stage grant is never presented for a commit, and the commit grant is exchanged after the validations of the Publish action have passed, as late as possible, so the authority to make a set permanent exists for the shortest time and only for a set that was staged, observed and validated.

## 2. The model

**What a channel declares.** Each repository of a channel carries a `PushIdentity`. `Federated` is the declaration of the identity the channel trusts to push:

| Field | Meaning |
|---|---|
| `subject` | the identity as the issuer states it, for a CI platform the `sub` claim of the job's identity token |
| `audience` | the audience the identity token must be issued for, which the channel's issuer checks |
| `environment` | the CI environment the publishing job runs in; a stage that publishes here has this `environment` (`machine.environment_mismatch`) |
| `exceptional_subject`, `exceptional_environment` | the identity and environment of an exceptional run off the mainline (5.6); a different identity, so the trust of an exceptional run is separate |
| `pinned_claims` | other claims of the identity token that the issuer pins (numeric ids, the workflow file, the ref, the event); ucdk compares them exactly and interprets none |
| `stage_scope`, `commit_scope` | the scope names the issuer grants for each phase; both are required and may be equal for an issuer that has one publish scope |

**ucdk may be stricter than the issuer and never looser.** Before any exchange, the grant source compares the claims of the job's own identity token with `subject`, `audience`, `environment` (or the exceptional pair) and `pinned_claims`; a difference is `GrantRefused(Denied)` with the first differing claim named, and the identity token is dropped unspent. The issuer remains the authority: passing the local check proves nothing to it.

**What is exchanged.** The CI platform gives the job a short-lived, signed identity token on request, for an audience the job names; every request returns a new token and the issuer accepts each one **once**. The grant source sends one identity token and the list of coordinates (names and versions) of the action, and the issuer answers a **grant**: a short-lived token that carries the scope and exactly those coordinates, acting as the identity the channel trusts. The grant is a `Grant` value that is neither `Debug` nor `Display`, cannot be cloned or serialised and zeroes on drop. It carries the expiry the issuer stated and nothing else ucdk reads. One grant covers one phase of one action: it is never presented in the other phase or for another action, and a grant that is expired or whose identity token was already used is replaced, never retried.

**What a run does, for a Publish action on a channel that stages.**

1. Every local check, including the claims comparison and `Publisher::grant_limit`, makes zero exchanges and zero calls if it refuses.
2. A `grant` record `Intent` (phase `stage`) is written; both held leases are renewed; the stage grant is exchanged; the answer is recorded.
3. The coordinates are staged; the stage grant is dropped.
4. The action's validations run on the staged set.
5. A `grant` record `Intent` (phase `commit`) is written; the commit grant is exchanged with a **new** identity token and the same coordinates; the answer is recorded.
6. The staged state is found by observation with the commit grant, the commit is made, the read-back is polled.
7. The commit grant is dropped when the action ends.

Before each call the engine does not use a grant whose stated expiry is not after the ledger's clock reading; it exchanges again first and records the exchange. A call that is refused for an expired or spent grant (`GrantRefused(Expired)` or `GrantRefused(AlreadyUsed)`) makes the engine exchange again, observe, and go on from what it saw, as many times as `Publish.attempts` allows (5.2).

## 3. Bootstrap

**Nothing standing.** ucdk holds no long-lived credential and no stored secret: there is none in the repository, the machine, the ledger or the store. It reads the CI platform's identity-token request variables in-process and nowhere else, and they never reach a child process, because a child gets only the environment the machine names (4.2).

**Bootstrap is the one-time step an administrator takes, with their own credentials, to create the trust on the channel.** ucdk has no verb that writes trust and never takes the administrator's credentials. What the administrator must create is exactly what `ucdk release trust --machine <file>` prints, per channel and repository: the channel, its kind and location, the artifact type, the `subject`, `audience`, `environment`, the exceptional pair if any, every `pinned_claims` pair in sorted order, the scope of each phase, and the artifact names the machine publishes. For a package registry this is: register the CI identity (those claims) as an allowed publisher on the registry, with the registry's own tooling; make sure the package names exist, or may be created by that publisher, as the registry requires; if the registry gates group or staged publication, enable it for the publisher; and protect the CI environment named by `environment` so that only the intended branch and reviewers can run in it. Then `ucdk release plan --check-credentials` exchanges a grant for each phase and discards it, and reports for each one of `MINTED`, `NOT_UNDER_PLATFORM` (the job has no identity token to give), `NOT_FEDERATED` (the push identity is `Anonymous`) or `REFUSED` with the issuer's code; only `MINTED` is a pass.

## 4. When a grant is refused, expires or was already used

**Between the stage and the commit.** The stage grant is dropped when the stage phase ends, and the commit phase does not use it, so its expiry between the two is invisible: the commit exchanges its own grant. The risk that remains is not an expired grant but a staged state that did not survive (a registry that expires an open staging area, or someone who aborted it). That is found by observation, never assumed: before the commit, every coordinate is observed with the commit grant, and a coordinate that is not `Staged` with the set's digest is `REFUSED` with `PUBLISH_STAGE`, nothing committed (5.2 step 2).

**Between the commit exchange and the commit call**, or during the read-back. The registry answers `GrantRefused(Expired)` (or the engine sees the stated expiry has passed). The engine writes a new `grant` record, exchanges a **commit-scoped** grant again with a fresh identity token, observes (`Staged` with our digest commits again; `Present` with our digest is read back; anything else is refused as in 5.2) and goes on. A stage-scoped grant is never substituted.

**An identity token already used.** An exchange that the issuer refuses because the identity token was spent (for instance a retry of an exchange whose answer was lost) is `GrantRefused(AlreadyUsed)`: the grant source asks the platform for a new identity token and exchanges again, within the same bound. Each exchange is its own `grant` record.

**What is recorded in the ledger.** Per grant, in the run ledger, a `grant` record named `<stage>/<action>/<phase>/a<attempt>-<n>` (3.3):

- **Before the exchange**, state `Intent`: `phase`, `channel`, `repository` (the artifact type), `subject`, `audience`, `scope`, and a digest of the sorted coordinates the grant is for. This says that an exchange may have been made, whatever its answer.
- **After the exchange**, state `Confirmed`: `outcome` `granted` with `expires_ms` (the issuer's stated expiry), or `outcome` `refused` with `fault` `denied`, `expired`, `used` or `unreachable`.
- **Never** the grant, the identity token, an issuer's token id or any value of the claims beyond the declared `subject`, `audience` and scope names; the record codec has no key for them (`codec::payload_keys_are_a_closed_list`), so no code path can write one. A `grant` record is audit data: a `Confirmed` one is never re-opened, and an exchange whose answer was lost stays `Intent` and is simply exchanged again by the next run (a token that was issued and never used expires).
- The `publication` records carry `phase` (`staging` or `committing`) while they are an `Intent`, so a reader knows which effect may have landed. The grant records say which authority was asked for; the publication records say what was done with it.

**What the run reports when the exchange is refused.** The action is `REFUSED` with `Reason.CREDENTIAL`, exit class 3, when nothing landed; it is `PARTIAL`, exit class 6, with `Reason.CREDENTIAL` when any coordinate is staged or committed, because those landed. The message names the phase, the channel and repository type, the scope, the `subject` and `audience` that were declared, the fault class and the issuer's own error code, and says what remains: for example `commit grant refused for channel registry, repository FILE: denied (oidc_not_trusted); subject repo:example-org/ucdk:environment:publish, audience packages.example.com, scope publish; nothing was committed; 2 coordinates remain staged (run again to commit, or run ucdk release abort)`. It never contains a token, and an error text that echoed any 16-byte run of one is withheld. The effects list has `STAGED` for each coordinate that is staged. Zero registry writes follow a refused exchange.

**Leftovers.** A job that dies between the stage and the commit leaves staged state, which holds registry resources until the registry's own deadline. The next run of the same stage observes it and goes on; a job that runs when the release failed gives it back with `ucdk release abort`, which exchanges a stage grant and calls `abort` for the coordinates that are `Staged` with the digest of the set, and never touches a committed one.

## 5. What this does and does not give

- **Enforced by the issuer.** That only the declared identity, in the declared environment, can obtain a grant; that a grant covers only the coordinates it was exchanged for and expires on its own; that an identity token works once.
- **Enforced by ucdk.** That the two phases use two grants; that a stage grant never reaches a commit; that a grant is never printed, cloned, logged or recorded; that nothing is exchanged before a local refusal; that a code-running action (a Build or Test action) never runs in a stage that can present a grant (`machine.publish_stage_runs_code`, section 6 of release.md).
- **A discipline, not a guarantee, on some issuers.** An issuer whose grants carry one publish scope for both phases (the two scope names are equal) does not stop a stage grant from committing: the separation then rests on ucdk's own use of the grants. The declaration says so by listing the same scope twice, and `ucdk release trust` prints that fact. An issuer that can mint stage-only grants makes the split enforceable by changing the declared scope names, with no engine change.
- **Short lifetimes are a fact of the issuer.** Grants live for minutes. An action whose phases take longer than one lifetime exchanges again; the bound is `Publish.attempts`, and a stage of very many coordinates is limited by `Publisher::grant_limit` and refused before any exchange when it exceeds it.
