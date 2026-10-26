# ucdk release: build, test, publish

Status: design. **Exists** means the code is on `main` today. **Decided** means a project decision that this document records. **Proposal** means a decision for review with no code yet. Sections 0 and 3.1 are Decided, and the statements in 3.1 about today's code are Exists (each was checked against `main` when this was written). Everything else is Proposal. Section 3 says, crate by crate, what stays and what goes, so the deletion pull request that follows this one is a mechanical list.

## 0. Project decisions recorded here (Decided)

- The deployment side of ucdk is deleted and stays recoverable from git history. There are no real cloud providers for now.
- The release process is the priority. ucdk gains BUILD, TEST (validations) and PUBLISH, with artifacts, release sets, channels and validation runs.
- Fakes come first. A fake builder, a fake registry and a fake validator are the executable specification, as the fake provider was for deployment.
- The release machine is not modelled on the deploy engine to keep that engine. It reuses what fits: the ledger with lease and fence, the write-ahead intent pattern for the one step that writes somewhere ucdk does not own (PUBLISH), the Kahn ordering, the length-framed hash, the C ABI pattern, the generator and the gates. Nothing else.
- Sequence: (1) this design, (2) one deletion pull request, (3) the release pull requests (section 11).

## 1. The model in one page (Proposal)

- A **release machine** is data: an ordered list of **stages**, each a list of **steps**. A step is a BUILD (turn declared artifacts into content-addressed files plus manifests) or a PUBLISH (put those files into a channel). Any step may carry **validations**: checks that must pass.
- An **artifact** is a file in a content-addressed store, described by a **manifest**: type, name, version, platform, sha256, typed metadata. The manifest is the only interface between a build and everything after it.
- A **release set** is the verified set of members one BUILD produced for one (revision, platform). Its **set hash** covers revision, platform and every member, and not who built it. BUILD and PUBLISH share one definition of the set, so they cannot disagree about it.
- A **channel** is a publish destination: per artifact type one repository (location, visibility, push identity). A published coordinate (channel, type, name, version, platform) is **permanent**. The seam has no delete and no overwrite. Withdrawal is an explicit `yank`, which keeps the bytes and marks the coordinate.
- A **validation** checks what a step made or published. It names its expected result rows up front. It passes only if at least one row is asserted and every asserted row passed. It carries a **validation run id**, stamped on everything it creates, journaled before creation and removable by that id.
- **Truth is observed.** The registry is truth for what is published. The ledger is the write-ahead record of what ucdk intended and confirmed. A publish whose answer was lost is resolved by reading the coordinate back. Correctness never depends on a run record.
- **One core, many hosts.** Rust core, C ABI, generated Python and TypeScript SDKs, the CLI. Builders, publishers and validators are traits, each with an in-memory fake, and each with a contract kit every real implementation must pass.

```
  Python SDK     TypeScript SDK                     ucdk CLI
       \               |                                |
        +---- C ABI (libucdk_capi, protobuf bytes) -----+
                       |
                  ucdk-capi   (the only unsafe crate)
                       |
                  ucdk-api    (ucdk.api.v1 <-> engine values; the one exit table)
                       |
                  ucdk-release-engine   run guard, write-ahead publish, resume
                   /          |            \
          ucdk-ledger   ucdk-release-seams   ucdk-release-fake
        (CAS, lease,    (Builder, Publisher,   (fake builder, registry,
         fence)          Validator, kits)       validator, faults)
                              |
                         ucdk-release (checked model, set hash, run hash, decisions)
                              |
                    ucdk-release-proto   ucdk-core
                    (ucdk.release.v1)    (paths, keys, hashes, Kahn order)
```

## 2. The protos (`ucdk.release.v1`, Proposal)

Everything crossing the ABI is protobuf bytes. The package sits beside the existing ones. Retired numbers are `reserved`. The generator reads the proto list in `ucdk_codegen::PROTO_FILES` (Exists; today two files), so the new file is one more entry. Enum zero values are `*_UNSPECIFIED` and are refused on read. Names (stage, step, validation, channel) match `[a-z][a-z0-9-]*`, at most 63 bytes, no trailing `-`. Artifact names also allow `_`.

Three names differ from their neighbours on purpose. `ucdk.api.v1` already has a message `Outcome`, an enum `ErrorCode` and an enum `CredentialSource` (all Exists, all deploy-shaped). The release package therefore says `Verdict`, `Reason` and `PushIdentity`, so a generated SDK never has two things of one name in one namespace.

```proto
syntax = "proto3";
package ucdk.release.v1;

// ---- declaration (authored; the SDKs build these) ----
message Machine {
  int32 schema_version = 1;          // 1. A newer major is a distinct refusal.
  string name = 2;
  repeated Stage stages = 3;
  repeated ArtifactDecl artifacts = 4;   // declaration order is part of the contract
  repeated Channel channels = 5;
  repeated string stamp_ignore = 6;      // path globs whose commits never raise the build number
}
message Stage {
  string name = 1;                   // also the unit of trust and of driver job
  repeated string needs = 2;         // earlier-declared stages; a DAG ordered by Kahn
  Trigger trigger = 3;
  string environment = 4;            // the trust scope the driver gives this stage; default = name
  bool exceptional = 5;              // may run off the mainline, with a reason (5.6)
  repeated Step steps = 6;           // run in this order; stop at the first not SUCCEEDED/NOOP
}
enum Trigger { TRIGGER_UNSPECIFIED = 0; RELEASE = 1; CHANGE_CHECK = 2; }  // CHANGE_CHECK: BUILD only
message Step {
  string name = 1;                   // unique in the stage
  oneof action { Build build = 2; Publish publish = 3; }
  repeated Validation validations = 4;   // any step kind may carry them
}
message Build   { string platform = 1; repeated string artifacts = 2; }
message Publish { string platform = 1; repeated string artifacts = 2; string channel = 3; }
// platform: "<os>-<cpu>" or "any". Never a cloud or a registry.

message Validation {
  string name = 1;                   // unique in the stage
  string validator = 2;              // a registered validator kind ("fake", later real ones)
  Subject subject = 3;               // what it reads
  repeated ExpectedRow rows = 4;     // authored up front; the denominator
  uint32 wait_polls = 5;             // CHANNEL_READBACK: bounded polls for absence; 0 = none
  map<string, string> options = 6;   // read only by the validator kind
}
enum Subject { SUBJECT_UNSPECIFIED = 0; LOCAL_SET = 1; CHANNEL_READBACK = 2; }
message ExpectedRow { string name = 1; string asserts = 2; }   // asserts: one sentence of what it proves

message ArtifactDecl {
  string name = 1;                   // byte-equal to its manifest name
  ArtifactType type = 2;
  Command builder = 3;               // executable (bare name or absolute path) and args; argv only
  repeated string needs = 4;         // artifacts that must exist first (a group names its members)
  reserved 5;
}
message Command { string executable = 1; repeated string args = 2; }
enum ArtifactType { ARTIFACT_TYPE_UNSPECIFIED = 0; FILE = 1; OCI = 2; CONDA = 3; PYTHON = 4; NPM = 5; }

message Channel {
  string name = 1;
  repeated Repository repositories = 2;   // at most one per artifact type
  bool monotonic = 3;                // never takes a build older than one it holds (5.6)
}
message Repository {
  ArtifactType type = 1;
  string location = 2;               // compared byte for byte; no two repositories share one
  Visibility visibility = 3;         // per repository, hence per artifact type
  PushIdentity push = 4;             // required; never defaulted
}
enum Visibility { VISIBILITY_UNSPECIFIED = 0; PUBLIC = 1; PRIVATE = 2; }
message PushIdentity { oneof kind { Anonymous anonymous = 1; Federated federated = 2; } }
message Anonymous {}
message Federated {
  string subject = 1;                // the one identity the registry trusts to push
  string audience = 2;
  string exceptional_subject = 3;    // used by an exceptional run; must differ from subject
}

// ---- produced (by a build; read by publish and validate) ----
message Manifest {
  int32 schema_version = 1;
  ArtifactType type = 2;
  string name = 3;
  string version = 4;
  string platform = 5;               // the release's, or "any"
  string file = 6;                   // bare name inside the store entry
  string sha256 = 7;                 // 64 lowercase hex of the file's bytes
  uint64 size = 8;
  oneof metadata { FileMeta file_meta = 10; OciMeta oci = 11; }  // typed per ecosystem
}
message FileMeta { map<string, string> labels = 1; }
message OciMeta  { string os = 1; string cpu = 2; string manifest_digest = 3; }
message Member { Manifest manifest = 1; string store_key = 2; }
message ReleaseSet {
  int32 schema_version = 1;
  string revision = 2;               // full commit id
  string platform = 3;
  repeated Member members = 4;       // sorted by name
  string set_hash = 5;               // recomputed on read; a differing value is refused
  ProducedBy produced_by = 6;        // never hashed
}
message ProducedBy { string run_id = 1; uint32 attempt = 2; }
message BuildStamp {                 // an input to a build; never typed, never read from the environment
  string version = 1; uint64 build_number = 2; string source_commit = 3; uint64 timestamp_ms = 4;
}
message Coordinate { string channel = 1; ArtifactType type = 2; string name = 3; string version = 4; string platform = 5; }

// ---- facts supplied by the driver ----
message RunIdentity {
  string run_id = 1;                 // [a-z0-9_-]{1,63}; never minted here
  uint32 attempt = 2;                // >= 1, required, no default
  map<string, string> context = 3;   // <= 32 pairs, recorded verbatim, gates nothing
}
message SourceFacts {
  string revision = 1;
  Tri on_mainline = 2;               // UNKNOWN is never a pass
  Tri is_tip = 3;
  string reason = 4;                 // 1..200 bytes, one line, for an exceptional run
}
enum Tri { TRI_UNSPECIFIED = 0; YES = 1; NO = 2; UNKNOWN = 3; }

// ---- results ----
enum Verdict {
  VERDICT_UNSPECIFIED = 0;
  SUCCEEDED = 1; NOOP = 2; SUPERSEDED = 3;          // exit class done
  REFUSED = 4; FAILED = 5; INDETERMINATE = 6;
  PARTIAL = 7; INTERRUPTED = 8; VALIDATION_FAILED = 9; CANCELLED = 10;
}
enum Reason {                        // stable; message text is for humans and never parsed
  REASON_UNSPECIFIED = 0;
  MACHINE_FORMAT = 1; MACHINE_VERSION = 2; STAGE_UNKNOWN = 3; SELECTOR = 4; SELECTOR_NO_MATCH = 5;
  SET_MEMBER = 6; SET_HASH = 7; CHANNEL = 8; CREDENTIAL = 9; APPROVED_NAMES = 10;
  PUBLISH_DIFFERENT_BYTES = 11; PUBLISH_YANKED = 12; PUBLISH_READ_BACK = 13; PUBLISH_UPLOAD = 14;
  NOT_ON_MAINLINE = 15; EXCEPTIONAL_REASON = 16; SUPERSEDED_BY_NEWER = 17; INPUTS_CHANGED = 18;
  VALIDATION = 19; CANNOT_TELL = 20; LEASE_HELD = 21; LEASE_LOST = 22; LEDGER = 23; BUILD_FAILED = 24;
  ROW_ACCOUNTING = 25; PIN_MISMATCH = 26; SOURCE = 27;
}
enum RowState { ROW_UNSPECIFIED = 0; ROW_PASSED = 1; ROW_FAILED = 2; ROW_NOT_RUN = 3; ROW_NOT_REACHED = 4; ROW_UNREADABLE = 5; }
message Row { string name = 1; RowState state = 2; string detail = 3; }
message ValidationReport { string name = 1; Verdict verdict = 2; repeated Row rows = 3; repeated string leftovers = 4; string validation_run_id = 5; }
message StepReport {
  string name = 1; Verdict verdict = 2; Reason reason = 3; string message = 4;
  repeated Effect effects = 5; repeated ValidationReport validations = 6;
}
message Effect { string what = 1; string subject = 2; EffectState state = 3; }
enum EffectState { EFFECT_UNSPECIFIED = 0; BUILT = 1; PUBLISHED = 2; ALREADY_PRESENT = 3; WOULD_BUILD = 4; WOULD_PUBLISH = 5; NOT_REACHED = 6; YANKED = 7; }
```

The request and response messages (`ReleaseCheckRequest`, `ReleasePlanRequest`, `ReleaseRunRequest`, `ReleaseStatusRequest`, `YankRequest`, `LeaksRequest`, their responses, and a generic `LedgerShowRequest` and `LedgerShowResponse`) are `ucdk.api.v1` messages owned by `ucdk-api`, as today. A run request carries the machine, run identity, source facts, stage, selectors and the approved run hash. The report is a pure function of the ledger and the engine's step results. No decision reads a report.

The set hash is SHA-256 over a domain tag `ucdk/release-set/1` followed by a newline (the convention `ucdk-plan`'s plan hash uses today, Exists), the revision, the platform, and for each member sorted bytewise by name: name, version, type, platform, sha256 and the sorted metadata pairs. Each field is framed by its 8-byte big-endian length, using the framing helper the plan hash uses today (`feed` in `ucdk-plan/src/hash.rs`, Exists; 3.1). Revision and platform are inside the hash so an approval of one commit or platform cannot be replayed on another.

Checked on read, each refusal named:
- An unknown enum value. A stage cycle, a stage needing itself or a later stage. A step or validation name used twice in a stage. A `CHANGE_CHECK` stage that holds a PUBLISH step, has `needs`, or is needed by another stage.
- An artifact `needs` cycle (every member of the cycle is listed) or dangling name.
- A channel with no repository, two repositories of one type in a channel, a location shared by two repositories (within or across channels), a repository with no `push`.
- `exceptional_subject` equal to `subject`, or set without `federated`.
- A PUBLISH step naming an undeclared artifact, or a channel with no repository for that artifact's type.
- A `CHANNEL_READBACK` validation on a BUILD step. A `LOCAL_SET` validation with no BUILD before it.
- A stage that holds a PUBLISH step whose `environment` differs from the environment the repository's `subject` names (6).

## 3. How it maps onto ucdk's crates

### 3.1 What stays and what goes (Decided)

**Stays (the framework the release machine reuses):**
- The ledger with lease and fencing: `MemoryLedger`, `FileLedger`, `Clock`, `SystemClock`, `ManualClock`, `Token`, `Snapshot`, the compare-and-swap write, the fenced write, the file format (atomic replace, checksum, newer-major refusal).
- The C ABI pattern in `ucdk-capi`: cdylib, header, opaque handles, status codes, panic containment, versioning, buffer ownership.
- The generator `ucdk-codegen` and the Python and TypeScript SDK loaders.
- The `xtask` gates and the per-crate 99% line and branch floor.
- `ucdk-api`'s outcome-to-exit pattern.

**Goes (one deletion pull request, right after this design, removes exactly this list and stays green):** the resource catalog (`Service`, `Job`) and the `ucdk.resource.v1` deploy kinds, descriptor lowering, the deploy verbs `plan`, `apply`, `destroy`, `status` and `leaks` (and the catalog-loading call that feeds them), the provider trait, the provider contract kit and the fake provider, plus their API messages, SDK templates and tests. `docs/architecture.md` is trimmed and marked historical where it describes deployment.

Per crate and module. "Moves" names the destination so the deletion cannot remove something the release side reuses.

| Crate or module | Verdict | What and why |
|---|---|---|
| `ucdk-proto` | **goes** | Holds only `resource.proto` and its generated types. Its `build.rs` recipe (`protox` plus `prost-build`, no `protoc`) is already copied in `ucdk-api/build.rs` and is reused by the new `ucdk-release-proto`. Its wire and descriptor test shapes (`tests/wire.rs`, `tests/descriptor.rs`) are re-created there. |
| `ucdk-provider` | **split** | Goes: the `Provider` trait, `Node`, `Kind`, `DesiredField`, `observe.rs` (`Live`, `Observation`, `Produced`, scopes), `stamp.rs`, `cell.rs`, `kit.rs`, the `Refusal` and `ProviderError` names in `error.rs`. Moves to `ucdk-core`: `ids.rs` (`valid_segment`, `NodePath` becomes `Path`, `NodeKey` becomes `Key`, `OwnerLabel`, `ModelError`, `SEGMENT_MAX`, the key scheme) with the path and key cases of `tests/model.rs` (`a_path_is_one_or_more_valid_segments`, `the_owner_is_the_first_segment`, `a_node_key_is_26_base32_characters_that_depend_on_the_whole_path`, `a_node_key_is_pinned_to_its_scheme`, `a_key_is_read_back_only_in_its_own_shape`, `identifiers_print_as_they_were_given`). Its `Digest` (a hex digest of a node's fields) is generalised into `Hash256`, and the seams spell the same type `Digest`. Kept as an idea only: the kit shape (`run_all` over a harness plus a self-test where a deliberately wrong stub must fail, `tests/kit_self_test.rs`) and the failure classification by what the caller may assume (refused, permanent, transient, in flight, unknown), re-expressed in 4.1 and 4.6. |
| `ucdk-plan` | **split** | Goes: `catalog/*` (lowering), `plan.rs`, `observed.rs`, `hash.rs::compute`, the tests including `tests/proto/queue.proto`. Moves to `ucdk-core`: `graph.rs::order` (Kahn, smallest-ready-first, returns the cyclic remainder) unchanged, with `tests/graph.rs`, and from `hash.rs` the hex hash type (`PlanHash` becomes `Hash256`, with `from_hex`), the length-prefixed `feed` and the domain-prefix convention, with the framing cases of `tests/hash_text.rs`. The set hash and run hash are built from these. |
| `ucdk-provider-fake` | **goes** | The deploy fake. Its fault-injection design (a call counter that fails at call N, read lag, ambiguous success, a foreign object, a race on the next call, and the ordered call log that records what the store really did) is the model for `ucdk-release-fake` (4.5), re-implemented for the new seams. |
| `ucdk-ledger` | **stays, generalised** | The CAS document, lease, fence, clocks, both ledgers, the file format and `sdk/ledger-golden/leased.ucdk` are what the release machine needs. It imports `ucdk_provider::{Kind, NodePath, OwnerLabel, PhysicalId, Produced}` today (`record.rs`, `format.rs`, `doc.rs`). The deletion PR replaces those with `ucdk-core::Path` and `Key`, a validated `Tag` string for `kind`, a `Locator` string for `physical`, and drops `Produced` (a record's `outputs` map stays as a plain string map, so the text format and the golden bytes do not change). `State::Reaping` and `Origin::Adopted` stay (today nothing writes `Adopted`; the release side does, 5.2); the record's `deps` and `outputs` lines stay in the format and release records leave them empty. The engine's lease lifecycle moves in as `LeaseGuard` (next row). |
| `ucdk-engine` | **goes; lease lifecycle moves** | `write.rs`, `delete.rs`, `status.rs`, `leaks.rs`, the plan-driven `Engine::handle`, the deploy `Mode`, `Request`, `Outcome`, `Refusal` and `EngineError` variants go. Moves to `ucdk-ledger::guard`: acquire after the refusals that need no ledger, renew at the start of every step (`engine.rs` renews before each step), release however the run ends, and the classification of an unconfirmed lease or release (lease held, lease lost, lease unconfirmed; unconfirmed counts as effect-possible). Its tests in `ucdk-engine/tests/lease.rs` move as `ucdk-ledger/tests/guard.rs`, driven by `ManualClock` against a counter instead of a provider (named in 11, step 2). The write-ahead rule (record intent before a mutation, confirm after, resolve a leftover intent by looking, as `write.rs` does) and the crash-sweep technique (`tests/crash.rs`: run again after stopping at call N) are kept as patterns and re-written against the release engine; the deletion commit message names the files so history finds them. |
| `ucdk-api` | **split** | Stays: the crate, `build.rs`, the single `ExitClass` enum and exhaustive `exit_class` match in `exit.rs`, `SessionConfig` (minus provider fields), the `Error` message and the session-level `ErrorCode` values (`INVALID_REQUEST`, `LEDGER`, `LEDGER_BUSY`), the wire-golden writer pattern (`tests/sdk_golden.rs`). Goes: plan, apply, destroy, status, leaks messages and conversions, `UnhostedKinds`, resource findings, the deploy `ErrorCode` values (their numbers become `reserved`), the provider `CredentialSource` enum with `SessionConfig.credentials` and `profile`. Gains: the release messages. |
| `ucdk-capi` | **stays, verbs replaced** | Stays whole: handle table, busy and poison rules, `catch_unwind` containment, `ucdk_buf`, status codes, ABI version, the header test and its `compile_error!` when built without unwinding. Of the ten functions today (`ucdk_abi_version`, `ucdk_session_open`, `ucdk_session_close`, `ucdk_buf_free` and six verb functions `ucdk_load_catalog`, `ucdk_plan`, `ucdk_apply`, `ucdk_destroy`, `ucdk_status`, `ucdk_leaks`), the six verbs and the `fake` provider registry go. The deletion PR adds exactly one verb so the ABI, ownership and panic suites still call something real: `ucdk_ledger_show`, a read-only view of a ledger document (token, fence, lease, records). The release verbs arrive later. |
| `ucdk-cli` | **stays, verbs replaced** | `args.rs` (no positional arguments), `exec.rs`, `render.rs`, `world.rs` structure and the "library plus thin main" rule stay. Goes: `plan`, `apply`, `destroy`, `status`, `leaks`, the fake world and its plan-text goldens. The deletion PR adds `ucdk ledger show` with a golden. |
| `ucdk-codegen` | **split** | Stays: `model.rs`, `names.rs`, `docs.rs`, `python/{message,types}.rs`, `typescript/{message,names,types}.rs`, the wire-golden contract, `golden_coverage.rs`. Goes: `kinds.rs` and `*/templates.rs` (typed templates and output references for resource kinds, which read `Resource.body` and the `portability` and `exposes` options) with `tests/templates.rs`. `PROTO_FILES` becomes `ucdk/api/v1/api.proto` only. |
| `xtask` | **stays, tables edited** | The verbs `check`, `coverage`, `fetch`, `gen-sdk` and `sdk-test`, and the modules behind `check` (`dco`, `spdx`, `runlines`, `lint`, `purity`, `direction`, `unsafe_scope`, `event`, `tree`), are unchanged. The `LAYERS` rows in `direction.rs` for `ucdk-provider`, `ucdk-plan` and `ucdk-provider-fake` go with those crates, the `ucdk-ledger` row stays, and the rows in 3.2 are added. |
| `sdk/python`, `sdk/typescript` | **stay** | Loader, version rule, `Session`, error classes, wire-golden tests. Generated trees are regenerated. Resource templates and the plan/apply scenario tests go; a ledger-show scenario replaces them. |
| `sdk/wire-golden`, `sdk/ledger-golden` | **stay, trimmed** | The resource and deploy-outcome goldens go (`service_internal`, `job_on_demand`, `resource_list`, the `plan`, `apply`, `destroy`, `status` and `leaks` requests, the `outcome_*` files, `catalog_response`). `session_config`, `error` and `map_defaults` stay, regenerated. `ledger-golden/leased.ucdk` stays. |
| `docs/architecture.md` | **trimmed** | The framework sections (C ABI, ownership, versioning, generation, ledger and lease) stay with deploy wording removed. Sections that describe the resource model, the engine and the deploy testing rows are marked historical with a pointer to git history. |

The deletion PR therefore removes exactly: the catalog and `resource.proto`, lowering, the deploy verbs, the provider trait, kit and fake, their API messages, SDK templates and tests, and `ucdk-engine`'s deploy code. It relocates what is named above and adds only `ucdk-core`, `LeaseGuard` and the `ledger show` verb.

### 3.2 Crates after the release work (Proposal; new in bold)

| Crate | Pure | Owns | Depends on |
|---|---|---|---|
| **`ucdk-core`** | yes | `Path`, `Key`, `Hash256`, framed hasher, Kahn `order`, `ModelError` | `sha2` |
| **`ucdk-release-proto`** | yes | `ucdk.release.v1` and generated types | `prost`; build: `protox`, `prost-build` |
| **`ucdk-release`** | yes | checked model, selection, set hash, run hash, `decide_publish`, `decide_supersede`, `account_rows`, state codec | core, release-proto |
| **`ucdk-release-seams`** | yes | traits `Builder`, `Publisher`, `Validator`, `ArtifactStore`, `CredentialSource`, `Source`, `Pause`; classified errors; the three kits | core, release |
| **`ucdk-release-fake`** | yes | in-memory store, builder, registry, validator, credential source, source; fault injection; call log | seams |
| `ucdk-ledger` | no | as today plus `LeaseGuard` and record helpers | core |
| **`ucdk-release-engine`** | no | `Engine::handle(Request) -> Outcome` for check, plan, run, status, yank, leaks; synchronous | all above |
| `ucdk-api`, `ucdk-capi`, `ucdk-cli`, `ucdk-codegen`, `xtask` | no | as in 3.1 | engine, ledger, seams as needed |

Direction rules enforced by `xtask` (`direction.rs`, `purity.rs`; Exists as mechanisms, with new rows): `ucdk-core`, `ucdk-release-proto`, `ucdk-release`, `ucdk-release-seams` and `ucdk-release-fake` never depend on `ucdk-ledger` or `ucdk-release-engine`. `ucdk-ledger` never depends on the engine. A pure crate depends only on pure crates. Pure crates hold no I/O, no wall clock and no `HashMap` (the purity template denies `HashMap`, `HashSet`, `SystemTime`, `Instant`, `File` and `TcpStream`), so every result comes out in one order.

The traits live beside the model, not in the engine, because kits must be callable by any seam implementation's tests without a dependency on the engine.

### 3.3 What each existing layer gains (Proposal)

- **Ledger.** New record kinds by `Tag`: `run`, `step`, `publication`, `validation`, `created`. A record's path is built from its tag and its `Key`, the key hashed from the readable coordinate, which is kept in `name`, because versions contain characters a path segment does not allow. Format minor 2 adds a `digest` tag, which an older reader ignores, and the states `yanking` and `yanked`. An older reader refuses a document holding a state it does not know (`format.rs` reports "not a state"), which is the right behaviour for a ledger a newer release engine wrote. It does mean the rule in the format header, "a minor adds tags, never changes one", is amended in the same PR to "a minor adds tags and state names; a reader refuses a state it does not know". The checked-in `leased.ucdk` header reads `ucdk-ledger 1.1`: the file written by minor 2 changes its header line, so that PR keeps the old file as a second golden that must still read.
- **Lease and fence.** One ledger document per channel (publication records; lease held by any publishing step on that channel) and one per run (run, step, validation and created records; lease held for the run). A BUILD-only run and a `CHANGE_CHECK` stage take no channel lease.
- **Engine.** A new engine on the same seam pattern (plain `Request` and `Outcome` enums, as `ucdk-engine/src/seam.rs` has today). It is not a reconciler: there is no desired-versus-observed graph, only an ordered machine and a record of where the run is. A step is `next = decide(machine, snapshot)` (pure) followed by executing `next` and writing the result. Only PUBLISH behaves like the old engine (5.2).
- **`ucdk-api`.** Release messages, and an `exit_class` match over the release `Verdict`. Every verdict is named, so a new variant does not compile until it has a class. The api's `Outcome` message keeps wrapping a verb's answer next to its `ExitClass`.
- **C ABI.** Release verbs with the existing signature: `ucdk_release_check`, `_plan`, `_run`, `_status`, `_yank`, `_leaks`, plus the kept `ucdk_ledger_show`. The ABI major is 0, and `architecture.md` section 5 promises nothing before 1 although it says removing a function bumps the major from then on. Decided here: the deletion PR keeps major 0 and moves the minor; the header records it. `SessionConfig` loses `provider`, `provider_options`, `cell`, `credentials` and `profile` (numbers reserved) and gains `builder`, `publisher`, `validator` (names selected from a registry, `fake` first), `channel_ledgers` and `run_ledger`. It keeps `holder` and the feature-gated `test_clock_ms`. Credentials never cross as bytes. Callbacks into the host are out of scope for v1.
- **Codegen and SDKs.** The same generator over `ucdk.api.v1` and `ucdk.release.v1`. Release declarations are plain generated messages. Typed templates for `ArtifactDecl`, `Repository` and `Validation` options are rebuilt from descriptors in the SDK PR, with no per-kind code.

## 4. Interfaces: builder, publisher, validator (Proposal)

All seams are synchronous, take `&mut self` and are `Send`. A session runs one call at a time. Every fallible method returns a classified error, never a bare bool or exception.

### 4.1 Error classification

```rust
pub enum SeamError {
    Refused(Reason),        // nothing happened; do not retry as is
    Permanent(String),      // will fail again; nothing happened
    Transient(String),      // retry is safe; nothing happened
    RateLimited(Backoff),   // retry after the stated number of Pause ticks
    InFlight(String),       // something may be underway; observe
    Unknown(String),        // the answer was lost; the effect may have landed; observe
}
```

`Refused`, `Permanent`, `Transient` and `RateLimited` promise that no external effect landed. `InFlight` and `Unknown` do not, and the engine resolves them by observing, never by assuming.

### 4.2 Artifact store and builder

```rust
pub trait ArtifactStore: Send {
    fn put(&mut self, bytes: &[u8]) -> Result<Digest, SeamError>;   // content-addressed, idempotent
    fn get(&mut self, d: &Digest) -> Result<Option<Vec<u8>>, SeamError>;
    fn has(&mut self, d: &Digest) -> Result<bool, SeamError>;
}
pub struct BuildRequest<'a> {
    pub decl: &'a ArtifactDecl,
    pub platform: &'a str,
    pub stamp: &'a BuildStamp,      // an input; the builder never reads git or the environment
    pub inputs: &'a [Member],       // verified members this one needs
}
pub trait Builder: Send {
    fn kind(&self) -> &str;
    fn plan(&self, req: &BuildRequest<'_>) -> Result<Vec<String>, SeamError>;   // no effect
    fn build(&mut self, req: &BuildRequest<'_>, store: &mut dyn ArtifactStore) -> Result<Manifest, SeamError>;
}
```

A builder writes bytes into the store and returns a manifest naming them. It cannot touch another member's entry. Run identity is not in `BuildRequest`: nothing run-specific reaches a member's bytes, so an identical rebuild has an identical set hash.

The engine, not the builder, verifies each member. One `verify_member` function checks that the manifest name equals the declaration byte for byte, the recomputed sha256 of the stored bytes equals the manifest's, the file is non-empty, the platform is the run's or `any`, and no entry beyond the manifest's file exists. It runs after each build, again before sealing (the second pass is the one hashed, so a later build that changed an earlier member is caught), and again by PUBLISH before it sends anything.

Artifacts build in Kahn order over `needs`, ties by name. A failing build leaves no manifest. A build that cannot be started is `INDETERMINATE`; one that exits nonzero, is signalled or times out is `FAILED` and names the artifact and the tail of its output. A shared stage budget hands each build the whole seconds left on an injected monotonic clock; under one second left, the build is not started and is reported not built.

### 4.3 Publisher (a registry)

```rust
pub enum Presence { Absent, Present { digest: Digest }, Yanked { digest: Digest } }
pub enum PutAnswer { Created, Duplicate }
pub trait Publisher: Send {
    fn types(&self) -> &[ArtifactType];
    fn observe(&mut self, c: &Presented, k: &Coordinate) -> Result<Presence, SeamError>;
    fn put(&mut self, c: &Presented, k: &Coordinate, file: &Blob<'_>, intent: &IntentId) -> Result<PutAnswer, SeamError>;
    fn yank(&mut self, c: &Presented, k: &Coordinate, reason: &str) -> Result<(), SeamError>;
    fn fetch(&mut self, c: &Presented, k: &Coordinate) -> Result<Option<Vec<u8>>, SeamError>;
    fn newest(&mut self, c: &Presented, ch: &str, name: &str) -> Result<Vec<Coordinate>, SeamError>;
    fn list(&mut self, c: &Presented, scope: &ListScope) -> Result<Vec<Listed>, SeamError>;   // may lag
}
```

There is **no delete and no force parameter**: the seam cannot express an overwrite. `observe` is a point read decided by fetching and hashing, and is authoritative. `list` may lag and is used only for reports (new names), never to decide an effect. A registry that exposes no digest in `observe` is refused as a channel home for that type: comparing identity field by field with no common field is a refusal, never equality. `Duplicate` is never an error to retry; the caller observes to tell "same bytes" from "someone else's". A transport failure on a mutating request is `Unknown`, because the caller cannot tell whether the bytes left.

### 4.4 Validator

```rust
pub struct ValidationRequest<'a> {
    pub decl: &'a Validation,
    pub validation_run_id: &'a str,          // stamp everything you create with it
    pub source: &'a mut dyn ArtifactSource,  // LOCAL_SET (store) or CHANNEL_READBACK (publisher; anonymous when public)
    pub members: &'a [Member],
    pub journal: &'a dyn CreatedJournal,
}
pub trait CreatedJournal { fn will_create(&self, kind: &str, id: &str) -> Result<(), SeamError>; }
pub trait Validator: Send {
    fn kind(&self) -> &str;
    fn run(&mut self, req: &mut ValidationRequest<'_>) -> Result<Vec<Row>, SeamError>;
    fn sweep(&mut self, validation_run_id: &str) -> Result<Vec<Leftover>, SeamError>;
    fn remove(&mut self, l: &Leftover) -> Result<(), SeamError>;   // only what the ledger names
}
```

A validator returns rows; the engine turns rows into a verdict with the pure `account_rows`. Emitted row names are compared with `decl.rows` index by index, because a count check alone passes a truncated run. A missing row, a swapped pair, a duplicate, an empty authored list and a blank `asserts` are each a fault naming the first divergent index. The verdict is `SUCCEEDED` only with at least one asserted row and every asserted row `ROW_PASSED`. `ROW_NOT_REACHED` and `ROW_UNREADABLE` are never a pass. `INDETERMINATE` means the validator could not read its subject. A spec fault is exit class 3, a failed row is class 7. Timeouts and skipped-by-request are step-level facts, not row states. A validator takes no secret.

### 4.5 Other seams and the fakes, first

```rust
pub trait Source {                       // version control, behind one seam
    fn stamp(&self, revision: &Revision) -> Result<BuildStamp, SeamError>;
    fn is_ancestor(&self, a: &Revision, b: &Revision) -> Result<Tri, SeamError>;
}
pub trait CredentialSource { fn present(&mut self, repo: &Repository) -> Result<Presented, SeamError>; }
```

`Source::stamp` takes the newest first-parent commit at or below the revision that touches anything other than the machine's `stamp_ignore` paths; the build number is the first-parent count up to it, so a documentation-only commit never raises it. A shallow history, a head that is not the revision, or a dirty tree is a refusal.

`ucdk-release-fake` is the executable spec. It is in-memory and deterministic, with no clock, I/O or `HashMap`.

- **`FakeBuilder`**: bytes derived from `(name, version, platform, args)`. Faults: `fail_at_call(n)`, `die_at_call(n)` (`Unknown`, a killed build), `hang`, `unstartable`, `write_foreign(name)`, `two_manifests`, `symlink`, `sha_mismatch`, `mutate_earlier_member()`, `unstamped()`.
- **`FakeRegistry`**: coordinate to bytes plus a yank mark, inspectable. Faults: `lose_answer_after_commit(n)`, `lose_answer_before_commit(n)`, `read_lag(n)`, `list_lag`, `present_different(coord, bytes)`, `race_next_put(bytes)`, `torn_put`, `rate_limit(n)`, `auth_refused`, `no_digest_exposed`, `unreadable_after(n)`. An ordered `commits()` and call log are asserted against.
- **`FakeValidator`**: a scripted row list. Faults: `truncate()`, `swap(i, j)`, `unreadable(row)`, `no_rows()`, `create_stamped_objects(n)`, `create_then_die`, `sweep_fails`.
- **`FakeCredentialSource`**: serves both source kinds and records every presentation, so tests assert zero presentations before a local refusal. `Presented` is neither `Debug` nor `Display`, zeroes on drop and cannot be cloned.
- **`FakeSource`**: tri-state answers including `UNKNOWN`; faults `shallow`, `dirty`, `head_mismatch`, `cannot_tell`.
- **Time.** `Pause` (counted polls and a backoff tick, never a sleep) and the ledger's `Clock` are the only time seams. Tests move a `ManualClock`.

### 4.6 Kits

`ucdk-release-seams::kit` holds `run_builder_kit`, `run_publisher_kit` and `run_validator_kit`, each a list of named scenarios written against the trait only. Every real implementation must pass them unmodified. A self-test feeds each scenario a deliberately non-conforming stub that must fail it, so a scenario that cannot fail does not ship.

- Builder: `same_request_same_bytes`, `manifest_name_equals_declared_name`, `sha256_is_of_stored_bytes`, `build_failure_leaves_no_manifest`, `builder_reads_no_environment`, `timeout_is_reported_not_hung`, `unstartable_is_not_failed`, `nothing_run_specific_in_the_bytes`.
- Publisher: `observe_absent_is_not_an_error`, `put_then_observe_returns_same_digest`, `put_of_present_same_is_duplicate_and_changes_nothing`, `put_over_present_different_leaves_old_bytes`, `no_digest_exposed_is_refused`, `yank_keeps_bytes_fetchable`, `yanked_coordinate_cannot_be_put_again`, `lost_answer_is_resolvable_by_observe`, `list_may_lag_but_observe_is_exact`, `public_read_needs_no_credential`, `credential_for_another_repository_sends_nothing`, `newest_lists_newer_builds_only`.
- Validator: `rows_follow_authored_order`, `truncated_run_is_a_fault`, `swapped_rows_are_a_fault`, `nothing_asserted_is_not_a_pass`, `unreadable_subject_is_indeterminate`, `created_is_announced_before_it_exists`, `created_objects_are_stamped_and_swept`, `sweep_twice_is_fine`, `validator_receives_no_credential`.

## 5. State, idempotency, crash and ambiguity (Proposal)

### 5.1 Principles

1. Observe before every effect. The ledger records intent before every effect. Nothing is trusted from an earlier read.
2. A coordinate is unique and permanent. Content identity is the digest, compared on exposed fields only.
3. Every local check happens before the first registry call or credential presentation. A refusal makes zero calls.
4. An ambiguity is never a pass and never a plain failure. It is `INDETERMINATE` or `PARTIAL` and says what to look at.
5. The run record is evidence and an optimisation. Correctness never depends on it.
6. Steps run in order and the run stops at the first step or validation that is not `SUCCEEDED` or `NOOP`. Later items are recorded `NOT_REACHED`, which never reads as a pass.

### 5.2 Publish of one coordinate (write-ahead)

Preflight, once per step and before any put: `verify_member` on every member; recompute the set hash and require it equal the sealed one; `observe` every member. Any member `Present` with another digest stops the step `REFUSED` (`PUBLISH_DIFFERENT_BYTES`, every such member listed, nothing sent). Every member present with our digest makes the step `NOOP`. Any failed observe makes it `INDETERMINATE`.

Then per coordinate, in Kahn order of the declared `needs`:

1. Resolve the credential once. A source that cannot serve this repository is a local fault before any request.
2. `observe`:
   - `Present` with our digest: write a `publication` record `Confirmed`, with `Origin::Adopted` when no intent of ours exists (someone published identical bytes), otherwise confirm the intent. Effect `ALREADY_PRESENT`.
   - `Present` with another digest: `REFUSED` `PUBLISH_DIFFERENT_BYTES`, never retried, never forced. `PARTIAL` if an earlier coordinate of this step landed.
   - `Yanked`: `REFUSED` `PUBLISH_YANKED`. A yanked coordinate is burned whatever its bytes.
   - observe fails: `INDETERMINATE`, nothing written.
   - `Absent`: continue.
3. Write the intent (`State::Intent`, digest, readable coordinate) under the fence.
4. `put`:
   - `Created`, `Duplicate`, `Unknown` or `InFlight`: go to 5. A duplicate with our digest is a converged earlier attempt.
   - `Transient`: retry within the declared attempt bound.
   - `RateLimited`: wait the stated `Pause` ticks, retry within the same bound.
   - `Refused` or `Permanent` (including authentication): the coordinate is `FAILED`, no effect landed. The intent is removed only after an `observe` shows `Absent`. The step stops at this coordinate and later ones are `NOT_REACHED` (open question 3 recommends this policy).
5. Read back: `observe`, polled up to a counted bound.
   - Our digest: record `Confirmed`; effect `PUBLISHED`.
   - Another digest: `PARTIAL`, `PUBLISH_READ_BACK`, needs a human, intent left, dependents not attempted.
   - Still `Absent`: `INDETERMINATE` (the registry may lag); intent left.

A dependent artifact (a group or index naming its members) is attempted only when every dependency is `Confirmed` and read back after the last member. A leftover intent whose coordinate is `Absent` on the next run is written again and re-put. Uniqueness of the coordinate at the registry makes that safe, so no stamp is needed to find "our" object. A step is done only when every coordinate is `Confirmed`.

### 5.3 The ambiguity table

| What happened | Local state | The next run | Verdict, exit |
|---|---|---|---|
| put answer lost, registry committed | intent | observe: same digest, confirm | `INDETERMINATE` 5, then `NOOP` |
| put answer lost, registry did not commit | intent | observe: absent, put again | `INDETERMINATE` 5, then `SUCCEEDED` |
| coordinate present with other bytes | none | refused again, always | `REFUSED` 3 (`PARTIAL` 6 if earlier ones landed) |
| read-back shows other bytes after our put | intent | same | `PARTIAL` 6 |
| registry index lags | intent | observe settles | `INDETERMINATE` 5 |
| process killed mid-step | run and step `Intent`, maybe a publication intent | lease lapses (plus skew); observe resolves | reads as `INTERRUPTED` 6 |
| lease lost to a newer holder mid-run | fenced out | the stalled run stops at its next ledger write; yanks and deletes nothing | `FAILED` 6 (`LEASE_LOST`) |
| a stale run (a newer tip exists) | none | stops before any effect | `SUPERSEDED` 0 |
| source facts unknown, history unreadable | none | refused, never guessed | `INDETERMINATE` 5 |

Fencing protects the ledger, not the registry: a `put` already in flight is not recalled. The intent record and the coordinate's uniqueness make that put resolvable by the next holder.

### 5.4 An interrupted run, and resume

A run is identified by the driver's run id and a required attempt (positive, no default, never minted here). The engine writes a `run` record `Intent` (reads as RUNNING) before the first effect and a `step` record before each step. If it cannot write the first one, the run is `FAILED` with nothing done. A rerun uses a higher attempt, takes the run's lease, and walks the stages in order:

1. A live lease held by another is `REFUSED` (`LEASE_HELD`), nothing changes. An expired lease is taken over after the skew bound (`MAX_CLOCK_SKEW` in `ucdk-ledger`, Exists), and the old holder's next write is refused as fenced.
2. The `run` record pins the machine hash, revision, platform and, once sealed, the set hash. A different machine, revision or platform on resume is `REFUSED` (`PIN_MISMATCH`). Sealed members are re-hashed against the set; a difference is `REFUSED`.
3. Each non-final record is resolved:

| Record found | Meaning | Resume does |
|---|---|---|
| build `Intent`, no sealed set | a build was cut short | discards that member's store entry from the run, rebuilds (builds are reproducible) |
| all builds confirmed, no sealed set | killed before sealing | re-verifies every member, seals |
| publication `Intent` | answer unknown | `observe` first, never `put` first (5.2) |
| validation `Intent` | cut short | sweeps its validation run id, then starts again with `k+1` (5.5) |
| `created` `Intent` | a resource may exist | sweeps it with the validation; never assumes absence |
| step `Confirmed`, same input digest (step spec, set hash, platform, revision) | finished | skipped and reported `NOOP`; a published coordinate is still observed once, so a channel changed meanwhile is seen |
| step `Confirmed`, different input digest | the run id is for one set of inputs | `REFUSED` `INPUTS_CHANGED`; use a new run id |

4. `status` reports a RUNNING record with no live lease as `INTERRUPTED` (exit 6) with the advice "run again to resume". The record is never rewritten to hide it.

With no records at all the same rules give the same end state, only slower. Resume is an optimisation and an audit trail.

### 5.5 Yank, retention and validation leftovers

- **No delete exists** in the trait, engine, ABI or CLI. It can be added later from history behind a new verb.
- **Yank** is explicit: `ucdk release yank --coordinate C --reason T --expect-hash H`. The engine writes `yanking`, calls `yank`, reads back `Yanked`, writes `yanked`. The bytes stay fetchable by exact coordinate, and re-publishing is refused. Yanking an `Adopted` publication takes the same flag and the record says it was adopted.
- **Retention** is "forever unless yanked". A registry that can only delete is refused as a channel home.
- **Validation run id.** `vr-` plus the first 12 hex digits of SHA-256 over (run id, stage, validation), then `-k`, where `k` is the start counter in the validation's record. It is at most 63 bytes, label-safe and deterministic for a resume. A resume of a cut-short validation sweeps `k` before starting `k+1`, so a validation never runs on top of its own leftovers. An operator re-run (`--only validation:<name>`) also takes the next `k`.
- **Journal first.** `CreatedJournal::will_create` writes a `created` record in `Intent`, fenced, before the validator creates anything. A validator that cannot journal does not create. Removal needs both the stamp and the record: `Reaping` intent first, record dropped only after a read shows the object gone. A stamped object that no record names is reported by `leaks` (exit class 8) and never removed on that evidence.

### 5.6 Admission, supersession, source policy

The driver supplies `SourceFacts`. The core reads no CI variable and runs no git outside `Source`. A stage that is not `exceptional` runs only when `on_mainline == YES`; `NO` is `NOT_ON_MAINLINE`; `UNKNOWN` is `INDETERMINATE`. An `exceptional` stage on a run off the mainline needs a one-line reason of 1 to 200 bytes, the revision equal to the one the driver started, and uses the repository's `exceptional_subject`. A release run is never a dry plan: `plan` and `run` are separate verbs. With `--admission`, a re-run (attempt above 1) of a first stage, or a first attempt whose revision is no longer the tip, stops `SUPERSEDED` before any record is created. A `monotonic` channel refuses a build older than one it holds: when `Source` says our revision is an ancestor of every newer build it is `SUPERSEDED`, when unrelated `REFUSED` (`SUPERSEDED_BY_NEWER`), when `Source` cannot tell `INDETERMINATE`, never `SUPERSEDED`. This applies to `plan` too.

## 6. Credentials (Proposal)

- **No standing credentials in v1.** A `PushIdentity` is `Anonymous` or `Federated`. The driver's platform issues a short-lived identity token, the source exchanges it for a short-lived push token for one repository, and it is held in a `Presented` value that cannot be printed, cloned or logged and zeroes on drop. A long-lived token is not a variant (Q6).
- **One writer per repository.** `push` is required, never defaulted, and no flag overrides it. A stage's `environment` must equal the environment the repository's identity names (the exceptional one for an exceptional run); otherwise the publish is `REFUSED` before any request. The identity token's environment claim is checked before the exchange.
- **Per surface and host.** A credential is presented only on the surface and host it was issued for; a mismatch is a local fault with zero requests. A public repository is read anonymously, so a read-back validation sees what a stranger sees.
- **Nothing secret crosses the ABI, the ledger or an error.** Configuration is flags. The source reads its platform's standard token variables in-process, nowhere else. An error is withheld when a server echoed the credential or any 16-byte run of it.
- **Trust split.** A `CHANGE_CHECK` stage holds BUILD steps only, has no environment and cannot be needed by any stage, so code under review never reaches a push identity. A `needs` edge is a data dependency; trust is per stage. Validators never receive a credential.
- `plan --check-credentials` mints and discards a token and reports `MINTED`, `NOT_UNDER_PLATFORM` or `NOT_FEDERATED`. The last two are never a pass.

## 7. CLI verbs and exit codes (Proposal)

Configuration is flags, with no positional arguments (as `ucdk-cli/src/args.rs` already requires, Exists) and no environment variables. Paths in a machine are relative to `--root`, not the working directory.

| Verb | What it does | Writes |
|---|---|---|
| `ucdk release check --machine <f>` | validate the machine; print stages and steps in order; every refusal names `<source>: line N` | nothing |
| `ucdk release plan --stage S [--only ...]` | what would build and publish, each coordinate's observed state (`WOULD_BUILD`, `WOULD_PUBLISH`, `ALREADY_PRESENT`), new names, the run hash | nothing |
| `ucdk release run --stage S --run-id R --attempt N --expect-hash H` | execute the selected steps and validations; create or resume the run | ledgers, store, registry |
| `ucdk release status` | read the ledger and observe each recorded and declared coordinate; say whether they agree | nothing |
| `ucdk release yank --coordinate C --reason T --expect-hash H` | explicit withdrawal | ledger, registry |
| `ucdk release leaks` | list stamped validation leftovers no record accounts for | nothing |
| `ucdk ledger show` | the ledger document as data | nothing |

Other flags: `--context key=value` (at most 32, untrusted), repeated `--only`, `--admission`.

**Selection and scope.** Selectors name what to run, never what to skip, so a step added later is not run by an old selective command. `--only step:x` runs the step without its validations. `--only validation:y` runs one validation alone against what is already there. Any selector makes the run `SELECTIVE`, and the report cannot present a selective run as `FULL`. A selector matching nothing is refused (`SELECTOR_NO_MATCH`) and lists the stage's steps and validations. The set hash in a report is non-empty only for a run that passed everything it selected. The final stderr line states scope and verdict.

**Exit table** (one function, `exit_class`, used by the CLI, the C ABI and both SDKs; today's table has the classes 0, 2, 3, 4, 6 and 8, Exists, and this adds 5 and 7):

| Class | Code | Verdicts |
|---|---|---|
| done | 0 | `SUCCEEDED`, `NOOP`, `SUPERSEDED` |
| usage | 2 | malformed command line, selector or run id |
| refused | 3 | `REFUSED`: nothing external changed |
| failed, no effect | 4 | `FAILED`: nothing landed; safe to repeat |
| indeterminate (new) | 5 | `INDETERMINATE`: cannot tell; never a pass |
| failed, changed | 6 | `PARTIAL`, `INTERRUPTED`, `CANCELLED`, lease lost or unconfirmed; run again to resume |
| validation failed (new) | 7 | `VALIDATION_FAILED`; re-validate with `--only validation:<name>` |
| leaks | 8 | `leaks` found unaccounted validation leftovers; a run that finds leftovers reports them and keeps its own class |

Codes 2, 3 and 4 promise that no external effect landed. A step record listing a landed coordinate or an announced created object can never carry one of them; the class is computed from the ledger. Retry advice derives from the class and a verb may make it stronger, never weaker. Exit numbers appear in one file, held by a test. Error ids are the closed `Reason` set, carried as `UCDK-E-<NAME>`, asserted by tests and never matched by message text.

## 8. Testing plan (Proposal)

Rules that bind every layer: tests are Rust (SDK tests in their own language, scoped to the binding); every crate on its own reaches 99% of lines and branches (`cargo xtask coverage`, Exists); a bug fix ships with a test that failed first; each test is documented by name and the behaviour it asserts; workflow `run:` lines are single `cargo xtask` verbs; **no test asserts timing**. Polls and backoff are counted through `Pause`; leases move through `ManualClock`.

| Layer | What is tested | How | Defect class it catches |
|---|---|---|---|
| `ucdk-core` | path and key grammar, hash hex rules, framing, Kahn order | unit; golden for key and framed hash text; order on cycles and unknown needs | a changed key or hash that orphans stored records; nondeterministic order |
| `ucdk-release-proto` | wire bytes, descriptor shape | golden bytes per message; descriptor breaking-change check; reserved numbers listed | a renumbered or retyped field |
| `ucdk-release` | machine validation, selection, set hash, run hash, `decide_publish`, `account_rows`, state codec | one unit test per refusal naming its rule; goldens for set-hash text; exhaustive tables of (presence, digest, yank, monotonic, history) to verdict; transition table total over every (state, event) | a rule silently not enforced; a set hash that includes who built it; a vacuous pass |
| seams and kits | the contract every implementation meets | kits on the fakes; kit self-test with non-conforming stubs | a kit scenario that cannot fail |
| `ucdk-release-fake` | fakes obey their own kits; each fault switch does what it says | kit plus per-fault tests and call-log tests | a fake that hides the defect it should expose |
| `ucdk-ledger` | CAS, lease, fence, format, guard | conformance on both ledgers; file torn, corruption, newer-major tests; guard tests on `ManualClock` | a lost update; a torn file read as a ledger; a stalled holder writing |
| `ucdk-release-engine` | publish rules (5.2), resume (5.4), yank, sweep, admission | integration over the fakes with call logs; crash sweep | a put before its intent; a delete; a different digest overwritten |
| `ucdk-api`, `ucdk-capi`, `ucdk-cli` | conversions, exit table, ABI, text | round trips; exhaustive match; in-process ABI conformance; CLI goldens over a world of fakes | an unmapped verdict; ownership, panic and version defects |
| SDKs and generator | binding and generated code | wire goldens, shared scenario files, end to end through the real library, `sdk-fresh` | an SDK that disagrees with the core or the other SDK; stale generated code |

**Crash and fault sweep.** Run a machine to completion on a fake world and count seam calls and ledger writes. Then for every N, run again stopping at call N (a kill modelled as the end of the run function with ledger and fakes kept), run once more to resume, and assert:
- the channel holds each coordinate at most once and only with the sealed digest;
- no record is `Confirmed` without a read showing it after;
- every `put` is preceded in the combined log by its intent write;
- every created resource has a `created` record written before the create;
- after a sweep no fake resource carries the validation run id;
- the final report equals the uninterrupted run's except for attempts.

It runs on the memory and the file ledger, and a second pass runs the same releases through `lose_answer_after_commit` at every put. Lease checks: a dead holder blocks the next run until the clock passes lapse plus skew; a holder whose clock moved past its lease is refused at its next write and changes nothing after.

**Gates are proven.** For each refusal in 5.2 and 5.6, a test asserts that the registry's commit log is empty and the credential source saw zero presentations.

**End to end.** One scenario set (machine, flags, expected class, golden text) is run in process by the CLI tests and through the built library by the Python and TypeScript tests, comparing the same expected values (the mechanism exists today for deploy scenarios: `ucdk-cli/tests/scenario.rs` and the SDK tests read the same data): `build_publish_validate`, `rerun_is_noop`, `resume_after_kill_during_publish`, `conflict_refused`, `validation_failed_exit_7`, `indeterminate_is_never_green`, `selective_run_says_so`, `superseded_runs_nothing`, `yank_then_republish_refused`, `leaks_reports_unrecorded_object`.

**Where it runs.**

| Suite | Pull request | After merge or on demand |
|---|---|---|
| `check` (fmt, clippy, deny, SPDX, run lines, direction, purity, unsafe scope, public-text lint, sign-off, sdk-fresh) | yes | yes |
| unit, golden, kits on fakes, crash sweeps, 99% floors (`coverage`) | yes | yes |
| ABI conformance in process, header and symbol check | yes | yes |
| SDK conformance (`sdk-test`) | Linux | all supported platforms |
| built `cdylib` loaded dynamically, Miri on pure paths, sanitizer builds, exhaustive sweeps on large machines | no | yes |
| a real implementation's kit against its own emulator (in its own crate) | yes | yes |
| a registry over the network | never | explicit flag, named account, no CI |

## 9. Design decisions and the defects they prevent (Proposal)

Each is a decision of this project, stated as behaviour with its mechanism.

- **One description of the machine.** The machine message is the only description of stages. The core reads no CI-vendor variable; a driver gets stage names, order, needs and environments as data from `check`. There is no second hand-written description to keep in agreement.
- **Durable progress and resume.** Run, step and validation records live in a leased, fenced ledger. A rerun skips confirmed steps with unchanged inputs and is correct without them.
- **Evidence is not authorization.** The gate between stages is a ledger fact written under the lease plus the set hash recomputed from the store. The report is derived data; the exit class is what a driver branches on.
- **Validations are not tied to one ecosystem or to PUBLISH.** A validator is a registered kind with typed options, attachable to any step, reading a local set or a channel read-back.
- **One publisher seam for every artifact type.** The kit forces every type through the same flow, so credentials and flows cannot diverge by type.
- **Permanence with explicit yank.** No delete, no overwrite, burned coordinates.
- **Visibility is per repository**, not per channel.
- **Ecosystem facts live in typed metadata arms.** The core compares names and digests only and keeps no copied tables.
- **A closed `Reason` enum**, never message matching.
- **The build stamp is an input** from one `Source` implementation, not derived in several places.
- **A missing store entry is a named refusal**, not an empty directory that happens to fail later.
- **Wire goldens and the minor rule exist from the first message.**
- **Every validation object is recorded before it is created** and swept after.
- **Run identity is supplied by flags**, recorded, never trusted, and gates nothing.

## 10. Open questions, each with a recommendation

1. **Machine text form.** Does the CLI read only binary protobuf, or also a text form? Recommend: binary plus canonical JSON through `prost-reflect`'s serde support behind one flag; people author through SDKs. Verify the dependency passes `cargo deny` in the model PR before taking it.
2. **Ledger scope.** One per channel plus one per run, or a single ledger? Recommend two kinds (3.3): a build never holds a channel lease and a change check never touches a channel. A write that spans both (step record, then publication record) is covered by the crash sweep.
3. **Failure policy.** Stop at the first failed coordinate or attempt every independent member? Recommend stop at the first (one rule); resume on rerun. Revisit with concurrency.
4. **Group artifacts.** Is "one group artifact last" a model rule? Recommend no: a group is an artifact whose `needs` name its members. A closure rule (every requirement names another member) is a validator, not a core rule.
5. **First artifact types.** Recommend `FILE` and `OCI` first, real publishers last. The enum names the other types now so protos do not change when they arrive.
6. **Registries without a federated identity.** Recommend not supported in v1. Add a reference variant later only with an explicit repository field, refusal of anything but a name, and a warning in `plan`.
7. **Where the stamp comes from.** Recommend one `Source` implementation that runs fixed git argv commands through a `ProcessRunner` seam (never a shell, scripted in tests), refusing a shallow clone or dirty tree.
8. **Builder provenance.** `ProducedBy` is recorded and not hashed. Recommend a signed provenance statement attached to the set later; not v1.
9. **Process builder.** Recommend argv only, no shell, output to log files, an injected monotonic clock for deadlines, a scripted runner for tests.
10. **More than one platform.** Recommend one run per platform in v1; a group hash and an approval binding it come with a second platform.
11. **First real registry.** Recommend a directory channel (no network, a complete `Publisher`, usable to validate before publishing), then OCI. Network clients wait for the dependency and TLS decision.
12. **Exceptional runs.** Recommend the field and the rules in 5.6 land with the model, but the verb path ships only when a real publisher exists.
13. **Ledger minor rule.** A new state name makes older readers refuse the document (3.3). Recommend accepting that and amending the format header's rule, over raising the format major, because the golden bytes of the existing layout then keep reading.
14. **ABI numbering at the deletion.** Recommend staying at major 0 with a minor bump (3.3), since section 5 of the architecture promises nothing before the first release.

## 11. PR sequence (Proposal)

Each PR keeps `cargo xtask check`, `coverage` and `sdk-test` green, names its tests, opens as a draft and is marked ready only after independent verification.

1. **This design** (`docs/release.md`, plus a pointer from `docs/architecture.md`; nothing else).
2. **Deletion and moves.** Removes exactly the 3.1 "goes" list. Moves `ids.rs`, `graph.rs` and the `hash.rs` pieces into the new `ucdk-core` with their existing tests, and the lease lifecycle into `ucdk-ledger::guard`. Re-points `ucdk-ledger` to core types. Trims codegen, api, capi, cli, xtask tables, SDK generated trees and goldens, and marks architecture sections historical. Adds only `ucdk_ledger_show` and `ucdk ledger show`. Tests kept or moved by name (existing names, checked on `main`): `ucdk-ledger/tests/{conformance,file,lease,golden,ledger}.rs` (`leased.ucdk` bytes unchanged); `core/tests/model.rs` from the six `ucdk-provider/tests/model.rs` cases listed in 3.1; `core/tests/order.rs` from `ucdk-plan/tests/graph.rs` (`a_node_comes_after_everything_it_needs`, `nodes_ready_together_come_out_in_name_order`, `an_empty_graph_is_ordered`, `a_need_outside_the_graph_is_ignored`, `a_cycle_returns_its_members_and_what_depends_on_them`, `a_node_that_needs_itself_is_a_cycle`); `core/tests/hash_text.rs` from `ucdk-plan/tests/hash_text.rs` (`plan_hash_reads_back_its_own_text`, `plan_hash_text_that_is_not_64_lower_case_hex_digits_is_refused`); `ucdk-ledger/tests/guard.rs` ported from `ucdk-engine/tests/lease.rs` (`second_writer_is_refused_while_another_run_is_in_progress`, `a_run_gives_the_lease_back_when_it_ends_even_when_it_failed`, `a_run_that_cannot_confirm_the_lease_is_not_safe_to_run_again`, `a_grant_whose_answer_was_lost_is_taken_again_at_once_by_the_same_holder_only`, `a_failed_release_after_a_failed_run_is_not_safe_to_run_again`, `a_failed_release_after_a_finished_run_leaves_the_run_done`, `a_run_whose_lease_was_taken_over_stops_at_its_next_write_and_the_new_holder_is_unharmed`, `a_run_that_lasts_longer_than_its_lease_keeps_it_by_renewing`, `a_fence_the_ledger_never_granted_cannot_write`, `errors_and_refusals_of_the_lease_read_as_sentences`); the two lease tests that name rollback or destroy (`a_lost_lease_is_not_rolled_back_over_the_new_holders_work`, `a_destroy_is_fenced_like_an_apply`) go with the deploy code; `capi/tests/abi.rs` over `ledger_show`; `api/tests/exit.rs` for the surviving classes.
3. **Proto and model.** `ucdk-release-proto`, `ucdk-release`: machine reading, set hash, selection, row accounting. Tests: `wire::every_field_round_trips`, `descriptor::retired_numbers_are_reserved`, `machine::unknown_enum_refused`, `machine::step_names_are_unique_in_a_stage`, `machine::change_check_stage_may_not_publish`, `machine::stage_cycle_is_refused_by_name`, `machine::artifact_needs_cycle_lists_members`, `machine::newer_major_is_a_version_error`, `selection::no_match_lists_choices`, `selection::any_selector_is_selective`, `set_hash::ignores_who_built_it`, `set_hash::changes_with_revision_and_platform`, `set_hash::member_order_does_not_matter`, `set_hash::golden_text`, `rows::truncated_run_is_a_fault`, `rows::swap_at_equal_length_names_both_rows`.
4. **Seams, fakes and kits.** Traits, errors, `ucdk-release-fake`, the three kits and their self-tests. Tests: every scenario in 4.6, `kit_self_test::each_scenario_fails_on_its_nonconforming_stub`, `fake_registry::lost_answer_after_commit_leaves_the_bytes`, `fake_validator::created_objects_are_listed_by_sweep`.
5. **Ledger records and states.** Record helpers, format minor 2 (`yanking`, `yanked`, `digest`), goldens (the minor 1 golden is kept and must still read). Tests: `format::older_reader_refuses_unknown_state`, `format::minor_2_round_trips_digest`, `format::minor_1_golden_still_reads`, `conformance::publication_record_survives_reopen`.
6. **Engine: BUILD, store, run record, resume.** Tests: `run::record_is_written_before_first_effect`, `run::unwritable_record_stops_failed_with_nothing_done`, `build::second_verification_catches_a_mutated_member`, `build::runs_in_needs_order`, `resume::confirmed_step_with_same_inputs_is_skipped`, `resume::changed_inputs_are_refused`, `resume::machine_changed_is_pin_mismatch`, `lease::second_runner_refused_while_lease_live`, `lease::fenced_runner_writes_nothing`, `crash::build_sweep_every_n`.
7. **Engine: PUBLISH.** The flow of 5.2, credentials, supersession. Tests: `publish::different_bytes_refused_nothing_written`, `publish::lost_answer_resolved_by_observe`, `publish::duplicate_with_our_digest_is_noop`, `publish::refusals_make_zero_registry_calls`, `publish::other_digest_midrun_is_partial`, `publish::concurrency_one_and_four_end_the_same`, `publish::monotonic_channel_unknown_history_is_indeterminate`, `crash::publish_sweep_every_n_with_lost_answers`.
8. **Engine: validations, sweep, yank, leaks.** Tests: `validation::vacuous_leg_is_not_a_pass`, `validation::run_id_is_deterministic`, `validation::restart_sweeps_previous_id`, `validation::object_is_recorded_before_it_is_created`, `sweep::stamp_without_record_is_reported_not_removed`, `yank::keeps_bytes_and_burns_coordinate`, `crash::validation_sweep_every_n`.
9. **API, C ABI, CLI.** Release messages and verbs, exit classes 5 and 7, ABI minor. Tests: `exit::every_verdict_has_a_class`, `exit::no_effect_classes_never_with_a_landed_record`, `exit::literals_appear_once`, `abi::release_verbs_follow_the_buffer_rules`, `abi::release_verbs_panic_contained`, `header::matches_source`, `cli::exit_codes_match_the_table`, `cli::selective_run_says_so_on_its_last_line`.
10. **Codegen and SDKs.** Release declaration templates from descriptors, wire goldens, shared scenario files and Python and TypeScript tests. Tests: `generate::release_package_is_covered_by_goldens`, `scenarios::release_end_to_end_matches_the_cli`.
11. **Process builder, local store, git `Source`** (real, argv only), each passing its kit.
12. **Directory registry and first real validator**, each passing the kits; live network runs only on demand. Later: OCI publisher, a network credential source, the exceptional-run path, a second platform.

## 12. Riskiest choices

1. Deleting `ucdk-engine` wholesale while moving its lease lifecycle into the ledger: the lease tests are the strongest proof of the fencing claims, and a thin move could silently lose a scenario. PR 2 ports them by name.
2. Generalising `ucdk-ledger` off the provider types while keeping the on-disk format byte-identical: the golden file is the only guard.
3. `ucdk_ledger_show` as the one addition in the deletion PR. If judged scope creep, the ABI suite has nothing real to call.
4. The seam has no delete and no overwrite, so a registry that supports only replace or delete is refused; some real registries may need an explicit exception later.
5. Two ledger kinds with writes that span both: the crash sweep must cover the gap between a step record and a publication record.
6. No standing credentials in v1 may exclude a registry a first user needs.
7. Resume keyed by an input digest refuses a legitimately edited retry; users must choose a new run id, and the error must say so.
8. Stop at the first failed coordinate favours simplicity over throughput.
9. Authored row accounting puts a burden on validator authors; the kit must make the failure obvious.
10. `Origin::Adopted` for identical bytes someone else published makes "ours" depend on bytes, not on who wrote them; yank needs the explicit flag for that reason.
11. Reading no CI variable pushes ref, run id and revision correctness to the driver; a driver that lies is detected only by the `Source` check of the revision.
12. A new ledger state name makes older readers refuse the document (question 13); an operator running an old binary against a new ledger gets a refusal, not a silent misread.
