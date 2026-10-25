# ucdk architecture

Status: design. This document says what exists in the repository today, and proposes how the
framework around it is laid out before more clouds and resources are added. Every section marks
which it is: **Exists** means the code is on `main`; **Proposal** means it is a decision to be
made in review and has no code yet. Where a claim about the code could rot, it names the crate
so a reader can check it.

## 1. The model in one page

- **One core**, written in Rust, built once as a shared library (`libucdk.so` on Linux,
  `libucdk.dylib` on macOS) with a small, stable C ABI.
- **Thin SDKs** in any language (Python and TypeScript first) load that library and call it.
  An SDK holds no planning, ordering, ledger or provider logic.
- **Resources are protobuf messages**: the `ucdk.resource.v1` protos
  (`crates/ucdk-proto/proto/ucdk/resource/v1/resource.proto`). Everything that crosses the
  library boundary is protobuf bytes.
- **Each SDK is generated from those protos.** Adding a resource to a cloud means adding a proto
  message and a provider implementation; no SDK source is written by hand for it (see 6.4 for
  the one place in the core that is not data-driven yet).
- **One state per environment.** The ledger of what a run owns is stored in the environment it
  deploys into, not on the machine that runs the tool.

```
  Python SDK     TypeScript SDK     (other SDKs)          ucdk CLI
       \               |               /                      |
        \              |              /                       |
         +---- C ABI (libucdk, protobuf bytes) ----+          |
                       |                                      |
                  ucdk-capi  (proposal; the only crate with unsafe code)
                       |                                      |
                  ucdk-api   (proposal; protobuf <-> engine values, one exit-code table)
                       |______________________________________|
                                       |
                                  ucdk-engine   write-ahead apply, closed-world delete
                                   /         \
                          ucdk-ledger       ucdk-provider  (trait, node model, contract kit)
                       (CAS document)        /          \
                                      ucdk-plan     providers: ucdk-provider-fake today,
                              (catalog, order, plan)       one crate per cloud later
                                      |
                                  ucdk-proto  (generated from the .proto files)
```

## 2. Layers and the crate that owns each

**Exists** unless marked. The arrows only go downward; `cargo xtask check` enforces part of this
(`xtask/src/direction.rs`: the provider, plan and fake crates never depend on the ledger or the
engine, and the ledger never depends on the engine) and the "pure" rule (a crate marked
`[package.metadata.ucdk] pure = true` holds no I/O, no clock and no `HashMap`, and depends only on
pure workspace crates).

| Layer | Crate | Owns | Pure |
|---|---|---|---|
| Wire catalog | `ucdk-proto` | the `.proto` files and the Rust types `build.rs` generates from them with `protox` and `prost-build`; no `protoc` binary involved | yes |
| Provider contract | `ucdk-provider` | the `Provider` trait, the node model (`Node`, `Kind`, `DesiredField`), stamps and cells, errors classified by what the caller may assume (`Refused`, `Permanent`, `Transient`, `InFlight`, `Unknown`), and the contract kit (`kit`) every provider must pass | yes |
| Planning | `ucdk-plan` | validating and lowering a `ResourceList` to nodes in dependency order (`catalog`), the plan against an observed state, and the `PlanHash` an approver pins | yes |
| Reference provider | `ucdk-provider-fake` | an in-memory provider with fault injection; the executable spec of the trait | yes |
| Ledger | `ucdk-ledger` | records, the single-document compare-and-swap `Ledger` trait, `MemoryLedger`; `Lease` is an interface only, nothing implements it | no |
| Engine | `ucdk-engine` | `Engine::handle(Request) -> Outcome`: write-ahead apply, re-observe before each mutation, closed-world delete, opt-in rollback, outputs, status. Synchronous | no |
| Host: CLI | `ucdk-cli` | `plan`, `apply`, `destroy` against the fake, as a library (`run(args, &mut World) -> u8`) with a three-line `main.rs` | no |
| Repository checks | `xtask` | `cargo xtask check`, `coverage`, `fetch` | no |
| API | `ucdk-api` (**proposal**) | protobuf messages for everything that is not a resource (session config, plan, report, status, findings, error) and the code converting them to and from the engine's values; the single table from outcome to exit class that both the CLI and the SDKs use | no |
| C ABI | `ucdk-capi` (**proposal**) | the `cdylib`, the C header, panic containment, buffer ownership. The only crate in the workspace allowed `unsafe` | no |
| SDKs | `sdk/python`, `sdk/typescript` (**proposal**) | the loader, the session wrapper, and generated typed templates | n/a |
| Generator | `ucdk-codegen` (**proposal**) | reads the descriptor set and writes the generated SDK sources | no |

The engine already states the seam this document builds on: the whole surface is one pair of
plain enums, `Request` and `Outcome`, "not a wire format: a later shared library wraps exactly
this seam and nothing else" (`crates/ucdk-engine/src/lib.rs`). The C ABI is that wrapper. It adds
no behaviour; any behaviour an SDK user can reach is one the CLI test suite can reach.

## 3. The C ABI (proposal)

### 3.1 Principles

1. **The ABI is small and carries bytes.** Types, verbs and fields change by adding protobuf
   fields, not by adding C types. The C surface is a handful of functions over opaque handles and
   `(pointer, length)` byte buffers.
2. **A domain outcome is not a C error.** A plan the engine refuses, a conflict, a failed apply
   with a partial report: these are `Outcome` values today (`Planned`, `Done`, `Refused`,
   `Failed`) and stay data in the response message. The C status code says only whether the call
   itself worked.
3. **No Rust type, no pointer into Rust memory, and no callback crosses the boundary** in version 1.
4. **No panic crosses the boundary** (5.3).

### 3.2 Functions

All functions are `extern "C"`, `#[no_mangle]`, prefixed `ucdk_`. `int32_t` return values are
status codes (5.1). `ucdk_buf` is `{ uint8_t *ptr; size_t len; }`.

```c
/* Version: returns (major << 16) | minor. Callable before anything else. */
uint32_t ucdk_abi_version(void);

/* Session: configuration is a SessionConfig message (provider, ledger, cell). */
int32_t ucdk_session_open(const uint8_t *config, size_t config_len,
                          ucdk_session **out_session, ucdk_buf *out_error);
void    ucdk_session_close(ucdk_session *session);          /* null is a no-op */

/* Verbs. Each takes a request message and fills a response message in *out. */
int32_t ucdk_load_catalog(ucdk_session *s, const uint8_t *req, size_t req_len, ucdk_buf *out);
int32_t ucdk_plan        (ucdk_session *s, const uint8_t *req, size_t req_len, ucdk_buf *out);
int32_t ucdk_apply       (ucdk_session *s, const uint8_t *req, size_t req_len, ucdk_buf *out);
int32_t ucdk_destroy     (ucdk_session *s, const uint8_t *req, size_t req_len, ucdk_buf *out);
int32_t ucdk_status      (ucdk_session *s, const uint8_t *req, size_t req_len, ucdk_buf *out);

/* Memory: every ucdk_buf the library fills is released here, once. Zeroed buf is a no-op. */
void    ucdk_buf_free(ucdk_buf buf);
```

That is eight symbols. The five verbs share one signature on purpose: a binding is written once.

What each verb is, in terms of what exists:

- `ucdk_load_catalog`: request is a `ResourceList`; response is the findings of `validate` and the
  lowered nodes summary (ids, kinds, dependency order). Pure: no provider, no ledger. Exists as
  `ucdk_plan::validate` and `lower`.
- `ucdk_plan`: request is a `ResourceList` and a mode (`APPLY` or `DESTROY`); response is the
  `Outcome` of `Request::Plan`: a plan (steps, hash) or a refusal. The plan crosses as a message
  that includes the `PlanHash`; the `Plan` struct itself is not serialized.
- `ucdk_apply` / `ucdk_destroy`: request is the `ResourceList`, the expected plan hash and (for
  apply) a rollback flag. The core re-plans from the world as it is now, refuses unless the hash
  equals the expected one, and then runs `Request::Apply` / `Request::Destroy`. This keeps the
  non-serializable `Plan` out of the ABI; the engine already refuses a plan that differs from the
  approved hash (`Refusal::PlanHashMismatch`, `Refusal::PlanChanged`), so the guarantee is the one
  the CLI has today (`--expect-plan-hash`).
- `ucdk_status`: request is a `ResourceList`; response is, per resource, the observed status
  (`Ready`, `Converging`, `Failed` as `ucdk_provider::Status`), its recorded outputs and whether the
  ledger and the cloud agree. Read-only. **This is new engine surface**: status exists today only
  as part of an apply report; a read-only `Request::Status` is added to the engine first (step 5
  in section 12).

A later verb (for example a leak sweep) is a new symbol and a new minor version; nothing existing
changes.

### 3.3 Handles and threads

A session owns one provider, one ledger handle and the cell identity. `ucdk_session` is opaque.

- The `Provider` trait takes `&mut self` and is `Send`; the engine is synchronous and stops at
  the first failure. A session therefore runs **one call at a time**. A second call on a busy
  session returns `UCDK_BUSY` at once; it does not queue. Different sessions are independent.
- A call blocks its thread until the engine returns. SDKs must not call it on an event-loop
  thread (6.5).
- There is no cancellation in version 1. A caller that gives up closes nothing mid-call; the
  engine's write-ahead design is what makes an interrupted run safe to resume, not the ABI.
- Closing a session with a call in progress is a caller error (`UCDK_BUSY` is returned by the
  call that observes it; close itself is not fallible). SDKs close from a finalizer only when
  idle.

### 3.4 How protos cross

Requests and responses are serialized protobuf in the proto3 binary format, exactly the format
`ucdk.resource.v1.ResourceList` already uses for `ucdk plan --input`. The caller owns request
bytes for the duration of the call only; the library copies what it keeps. Responses are bytes
allocated by the library.

Resource messages stay in `ucdk.resource.v1`. Everything else is a new package, `ucdk.api.v1`
(`SessionConfig`, `PlanRequest`, `PlanResponse`, `ApplyRequest`, `Report`, `StatusResponse`,
`Finding`, `Error`), owned by `ucdk-api` and compiled by the same `protox` + `prost-build` path
as the catalog. The generator (section 6) reads both packages, so SDK types for requests and
responses are generated too.

`SessionConfig` is data, not environment variables: provider name and its options, ledger kind
and location, and the cell identity. **Credentials never cross the ABI as bytes.** The config
names a credential source (a profile name, the platform's default chain, or an identity the
provider resolves itself); the provider reads it in-process.

### 3.5 Providers and callbacks

Version 1 has **no callbacks**. Providers are Rust crates linked into the core (section 8).
Reasons: the `Provider` contract is synchronous with a `&mut self` receiver and a strict error
classification; a foreign-language provider would re-enter the host runtime from inside the
engine's write-ahead sequence, where a host exception or a GIL/event-loop stall corrupts the
reasoning the engine relies on ("this call returned `Unknown`, so observe"). If a loadable
provider is wanted later, it is a second, separate ABI with the same buffer conventions, versioned
independently (section 8.3). It is not a function-pointer table added to this one.

## 4. Memory ownership and error model across the boundary (proposal)

### 4.1 Ownership rules

| Thing | Allocated by | Freed by | Lifetime |
|---|---|---|---|
| request bytes | caller | caller | borrowed for the call; the library never retains the pointer |
| response / error `ucdk_buf` | library | caller, with `ucdk_buf_free` | until freed; freeing twice or with a foreign `free` is undefined |
| `ucdk_session` | library (`ucdk_session_open`) | `ucdk_session_close` | until closed |
| strings inside messages | inside the protobuf bytes | with the buffer | n/a |

Rules that make this testable: the library allocates and frees with the same allocator (the
`ucdk_buf_free` function exists so the caller never calls `free`); on any non-zero status,
`*out` is either zeroed or holds an `Error` message, never partial garbage; a null pointer with
a zero length is a valid empty request; a null pointer with a non-zero length is
`UCDK_INVALID_ARGUMENT`; lengths are `size_t` and are checked against a documented maximum
(`UCDK_ERR_TOO_LARGE`) before decoding.

### 4.2 Status codes

`0` is success. Non-zero codes are about the call, not about the world:

| Code | Name | Meaning |
|---|---|---|
| 0 | `UCDK_OK` | `out` holds the response message (which may itself say refused or failed) |
| 1 | `UCDK_INVALID_ARGUMENT` | null where not allowed, bad length, bad handle |
| 2 | `UCDK_DECODE` | the request bytes are not the expected protobuf message |
| 3 | `UCDK_BUSY` | the session is in use |
| 4 | `UCDK_ABI` | the session config asks for something this library version does not have |
| 5 | `UCDK_TOO_LARGE` | a request over the size limit |
| 100 | `UCDK_PANIC` | an internal panic was contained; the session is poisoned and must be closed |
| 101 | `UCDK_INTERNAL` | an invariant the library checks failed |

On a non-zero code the library tries to fill `out` with an `Error` message (`code`, `message`,
optional detail). The message is for a human and is not parsed. This extends rather than replaces
the exit table the CLI documents today (0 done, 2 usage, 3 refused with nothing changed, 4 failed
with no effect, 6 failed after changing something): `ucdk-api` holds the one function from
`Outcome` to that exit class, the CLI prints it as its exit code, and each SDK returns it as a
field of the result. There is one table, not one per host.

### 4.3 No panics across FFI

Every exported function body runs inside `std::panic::catch_unwind`. The workspace forbids
`unsafe` today (`unsafe_code = "forbid"` in the root `Cargo.toml`); the C ABI crate is the single
exception, by a per-crate lint override that `cargo xtask check` pins (it fails if any other
crate sets it, and if the override lives anywhere but that crate's manifest). The `cdylib` is
built with `panic = "unwind"` so containment works; a contained panic poisons the session (the
engine may be mid-mutation, and the write-ahead ledger, not in-memory state, is the recovery
path) and returns `UCDK_PANIC`. Panics in a destructor, and a panic while the panic message is
being built, abort the process by Rust's own rule; the tests in 10.3 pin that this path is not
reachable from the ABI.

## 5. ABI versioning and compatibility (proposal)

- **Two numbers.** `ucdk_abi_version()` returns `(major << 16) | minor`. The generated header
  defines `UCDK_ABI_MAJOR` and `UCDK_ABI_MINOR`. Adding a function or a verb bumps the minor;
  removing or changing the signature of an existing function, the status-code meanings, or the
  ownership rules bumps the major. The major starts at 1 on the first release; before that the
  library is `0.x` and nothing is promised.
- **Loader rule.** An SDK built for `(M, m)` accepts a library with the same `M` and a minor
  `>= m`. It checks this first thing after loading, before any other call, and fails with a clear
  error naming both versions. A newer SDK against an older library is refused, never guessed at.
- **Payload compatibility is the protobuf rules.** Fields are added with new numbers; a removed
  field's number and name are `reserved` (the catalog already does this for `only_in`); an
  unknown field is ignored on read. A change that breaks that is a new package version
  (`ucdk.resource.v2`) that coexists with `v1` in the same library; `v1` messages keep working
  until the major version that drops them.
- **A breaking-change check on the protos** is added to `cargo xtask check`: it compares the
  descriptor set against the one recorded for the last release and fails on a removed or
  renumbered field, a changed type, or a removed enum value. It is a Rust program over
  descriptors (no extra tool).
- **The plan hash is part of the compatibility surface.** The hash bytes of existing nodes are
  pinned by goldens in `ucdk-plan` and the CLI test data. A change to them is a deliberate,
  separately reviewed change, because users pin hashes between `plan` and `apply`.
- **Symbols.** The library exports only `ucdk_*` symbols (a version script on Linux, an exports
  list on macOS), and a test reads the built library's exported symbol table and compares it with
  the header.

## 6. How SDKs are generated (proposal)

### 6.1 Input

The descriptor set for `ucdk.resource.v1` and `ucdk.api.v1`, produced in-process by `protox` with
source info (comments) included. This is the same compile `ucdk-proto/build.rs` already does, so
no `protoc` and no per-language protobuf toolchain is needed to generate anything.

### 6.2 What is generated, and what is hand-written

| Generated, never edited | Hand-written, small and fixed |
|---|---|
| message classes and enums for both packages, with docs taken from the proto comments | the C ABI binding: load the library, check the version, call the eight functions |
| binary encode and decode for those messages | the `Session` type: `open`, `load_catalog`, `plan`, `apply`, `destroy`, `status`, `close`, and the error type built from the status codes |
| one typed template per resource kind (the fields of the `Resource.body` oneof): required fields as constructor arguments, optional ones as keywords, oneofs as separate named constructors (an image is `Image.digest(...)` or `Image.step_output(...)`, never a free-form dict) | locating the library (6.5) |
| typed output references from the `exposes` option: `service.url` is a `Ref`, not a string | async wrapper (TypeScript) |
| the portability marker per kind (`PORTABLE` or `CLOUD_BOUND`) as metadata | packaging metadata |

The hand-written part does not grow when a resource is added. A test enforces that (10.4).

### 6.3 Encoding: generated, no protobuf runtime dependency (recommendation)

Two ways to serialize the generated messages:

- **A.** Generate the message code with each language's own protobuf tooling and depend on its
  runtime. Least code of ours; adds a runtime dependency, a per-language generator toolchain and
  version skew to each SDK.
- **B.** The generator emits the encoder and decoder itself. The used subset of proto3 is small
  (strings, `uint32`, bool, enums, nested messages, repeated, `map<string, Message>`, `oneof`,
  `optional`, `Duration`). No runtime dependency in the SDK, one generator in Rust, tested in
  Rust.

Recommendation: **B**, with one cross-language contract test: for each golden message in
`crates/ucdk-proto/tests/wire.rs`, every SDK encodes the same value and must produce the same
bytes, and decodes the golden bytes back to the same value. If B proves too costly in review, A
is the fallback and the contract test is unchanged.

### 6.4 What "adding a resource" costs today, honestly

The stated goal is "a proto and a provider implementation, nothing else". The code today does not
yet meet that: **lowering is hand-written per kind**. `ucdk-plan/src/catalog/read.rs` matches on
`Body::Service` and `Body::Job` and `render.rs` has one function per kind to turn them into
`DesiredField`s. A new proto message therefore also needs lowering code in `ucdk-plan` today.

Proposal: make lowering descriptor-driven. A kind's fields are read from the message by
reflection over the same descriptor the generator uses, using field options in `resource.proto`
for what is not implied (which fields are references, which are secrets, which form the
identity). Then a new resource is: a message in a `.proto`, field options on it, and a provider
that implements the kind. The existing `portability` and `exposes` message options show the
pattern. This is a prerequisite step of the sequence (section 12, step 4), done on `service` and
`job` first with the existing plan goldens unchanged as the proof.

### 6.5 How an SDK finds and loads the library

- The library ships **inside the SDK package**, one per platform: a platform wheel for Python and
  an optional per-platform package for TypeScript, covering Linux x86-64, Linux arm64 and macOS
  arm64. The loader looks in the package directory first.
- An explicit `library_path` argument to the constructor overrides it (for tests and for a
  vendored build). There is no environment-variable lookup: configuration is arguments.
- Python binds with `ctypes` (no compiled extension, so one pure-Python wheel plus a data file per
  platform). TypeScript binds through Node's native addon interface or a pinned FFI package;
  which one is decided in the TypeScript PR, with the rule that it supports running a blocking
  call off the main thread.
- Blocking calls: the Python SDK releases the GIL by construction under `ctypes`; the TypeScript
  SDK runs each call on a worker thread and exposes a promise.

### 6.6 Generated code in the repository

Proposal: generated SDK sources are **not committed**; `cargo xtask gen-sdk` writes them into the
SDK package directories and the SDK tests and packaging jobs run it first. The generator's own
output for a fixed small proto (a checked-in fixture) is committed as a golden, so a generator
change shows up as a diff in review. This follows the repo's existing rule that nothing generated
is committed (`ucdk-proto/build.rs`).

## 7. Resources as protos (exists, with the gap in 6.4)

`ucdk.resource.v1` defines `Resource { id, uses, oneof body { Service, Job } }`, `ResourceList`,
`Ref` (a resource and one of its outputs, standard or named), `Value` (literal, parameter or
ref), `Image` (a step output or a digest, plus platform), `SecretRef`, and the message options
`portability` and `exposes`. The lowering produces, for each resource, a `run` node and one extra
node of the type's other role. Plan hashes are computed over the lowered nodes, so a change to
lowering is a change to hashes and is gated by goldens.

A kind that is only available on one cloud is `CLOUD_BOUND`; the portable ones are mapped by each
provider. A provider reports which kinds it hosts; the CLI today refuses a plan for a kind the
chosen provider does not host (`refused: kind job is not hosted by fake-limited`), and the same
refusal is a response field in the SDKs.

## 8. Providers (exists as a Rust trait; loadable providers are a proposal)

### 8.1 Built in

A provider is a Rust crate implementing `ucdk_provider::Provider`: `observe`, `create`, `update`,
`delete`, `list_owned`, `list_stamped`, plus `state_class`, `deletability` and `stamp_mode` per
kind. Contract points the engine depends on are listed on the trait and checked by `kit`: a create
writes the stamp in the create call, an existing object of that name is refused as
`AlreadyExists` and left alone, `delete` of a missing object succeeds, and `observe` reports
`Failed` or `Converging` as the cloud has it. `kit::run_all` is the conformance suite; a provider
that does not pass it is not a provider.

The core links the providers it ships. `SessionConfig.provider` selects one by name from a
registry owned by the provider crates (the CLI's closed enum of `fake` and `fake-limited` in
`ucdk-cli/src/world.rs` is the shape this replaces). Cloud product vocabulary stays inside that
cloud's provider crates.

### 8.2 One crate per cloud, emulator first

Each real cloud is two crates: the provider, whose mapping logic runs against a narrow API trait
of its own, and an emulator of that API with lag and ambiguous-outcome faults. The provider must
pass the unmodified `kit::run_all` on the emulator, and meets the coverage floor from the
emulator alone. The thin adapter that calls the real service is a third, separate crate, so the
provider's floor does not depend on a network. Live runs against a real account are behind an
explicit flag and are never part of CI.

### 8.3 Loadable providers

Out of scope for version 1 (3.5). It stays a proposal until someone names a provider that cannot
be written in Rust.

## 9. State and the ledger

**Exists:** `Ledger` is a single document with a version `Token`; a write names the token it read
and fails with `LedgerError::Stale` if another writer got there first. `MemoryLedger` is the only
implementation, so today nothing survives the process. `Lease` (acquire, release, `Epoch`) is an
interface and nothing implements or calls it. The engine copes with two runs on one ledger by
compare-and-swap writes and the create refusal, not by exclusion.

**Decided direction:** deployment state is stored in each environment. The ledger for a cell
lives in storage the provider owns inside that environment (an object store in the same account,
project or subscription; for on-premises, a store the operator names), so that whoever can reach
the environment with the right credentials sees the same state, and no machine-local file is the
source of truth.

**Proposals, in order:**

1. A file ledger (single machine; documented as not a team backend) so the CLI and SDKs can resume
   after a crash before any cloud exists. Atomic replace, restrictive file mode, refuses corrupt,
   truncated or newer-major documents.
2. A real lease with an injected clock and epoch fencing in the ledger write, so a stalled run
   cannot write after a newer one took over. Fencing protects the ledger, not the cloud.
3. An object-store ledger on conditional writes (create-only, then if-match), with the lease in
   the same document, implemented by the provider that owns the environment. A provider that
   cannot give conditional writes is refused as a ledger home.

The ledger holds no secrets; secret outputs are handed to the run that asks and never written.
`SessionConfig` names the ledger by kind and location, plus a credential source by name.

## 10. Testing plan

Rules that already bind every layer: tests are Rust, a bug fix ships with a test that failed
first, and `cargo xtask coverage` requires every crate on its own to reach **99% of lines and 99%
of branches** (generated code and a `main.rs` excluded). A rule below that goes beyond what the
repository does today is a **proposal**.

### 10.1 Per layer

| Layer | What is tested | How | Defect class it catches |
|---|---|---|---|
| `ucdk-proto` | wire bytes of every message; descriptor shape | golden bytes (`tests/wire.rs`), descriptor tests (`tests/descriptor.rs`); proposed: breaking-change check against the last release's descriptor set | a renumbered or retyped field that silently changes what old callers send |
| `ucdk-provider` | the model: names, ids, stamps, digests | unit tests; the kit has its own self-test where a deliberately nonconforming stub must fail each scenario (`tests/kit_self_test.rs`) | a kit scenario that cannot fail |
| `ucdk-plan` | validation findings, lowering, ordering, plan and its hash | unit and golden tests (`tests/catalog.rs`, `graph.rs`, `plan.rs`, `hash_text.rs`) | a changed hash, a missed cycle or dangling reference, a foreign object not refused |
| providers | the trait contract | `kit::run_all` on each provider; fake: fault injection (`fail_at_call`, `read_lag`, `ambiguous_success`, `foreign`, `race_next`, `create_failed`, `converging_reads`) | create that does not stamp atomically, an ambiguous result treated as failure, a delete of a foreign object |
| `ucdk-ledger` | CAS semantics, record encoding | unit tests; proposed: one conformance suite run against every implementation | lost update, stale write accepted |
| `ucdk-engine` | apply, destroy, resume, rollback, outputs, status | integration tests over the fake with call logs (`tests/apply.rs`, `crash.rs`, `delete.rs`, `outputs.rs`, `status.rs`) | a mutation before its intent is recorded, a record dropped after a failed read, a delete of an object that is not ours |
| `ucdk-cli` | arguments, exit codes, plan text, end to end | in-process runs over a `World` (`tests/cli.rs`, `e2e.rs`, `status.rs`) with plan-text goldens | wrong exit class, a drifting plan text |
| `ucdk-api` (proposal) | message-to-value conversion in both directions; the outcome-to-exit table | round-trip tests; an exhaustive match so a new `Outcome` or `Refusal` variant fails to compile until it is mapped | a field dropped in conversion, an unmapped outcome |
| `ucdk-capi` (proposal) | the eight functions, called as C would | see 10.3 | ownership, panic, version and malformed-input defects |
| SDKs (proposal) | binding and generated code | see 10.4 | an SDK that disagrees with the core or with another SDK |
| generator (proposal) | output for fixture protos | golden output; compile or import check per language | generated code that does not load or encode wrongly |

### 10.2 Crash and fault injection

The engine's central claim is that a crash at any point leaves a state the next run resolves by
looking. It is tested by running a second `run()` over the same world after stopping the first at
call N, for every N of a plan, against each fault the fake can inject, and asserting: at most one
object per node, no record dropped without a read showing the object gone, no delete of an object
without both stamp and record. The same sweep is run again for each real provider's emulator
(8.2). Proposed additions when the lease exists: advance the injected clock past the lease time
between the two runs and assert the old holder's write is refused as fenced.

### 10.3 ABI conformance (proposal)

Run from Rust, in-process, against the exported functions (so they count toward coverage) and
again against the built `cdylib` loaded dynamically:

- every function with null, zero-length, oversized and truncated inputs; a handle used after
  close; a double close; `ucdk_buf_free` on a zeroed buffer;
- a provider that panics (a test-only provider behind the same registry) must give `UCDK_PANIC`
  and a poisoned session, never an abort or an unwind into the caller;
- an ownership check: a counting allocator shows every response buffer is freed by
  `ucdk_buf_free` and nothing leaks over a loop of calls;
- a version check: a faked lower minor and a different major are refused with `UCDK_ABI`;
- the exported symbol table of the built library equals the header's declarations;
- run under Miri for the ABI crate's pure-Rust path after merge, and under a sanitizer build
  after merge, since neither runs in the pull-request budget.

### 10.4 SDK conformance (proposal)

The SDK tests are written in the SDK's language, because the point is to exercise the binding.
This is an exception to "tests are Rust" in `CONTRIBUTING.md` and needs that rule amended
explicitly, scoped to `sdk/<language>` and driven only through a `cargo xtask sdk-test` verb so
the workflow still runs one verb per line.

- **Wire contract:** every golden message encodes to the same bytes in every SDK and decodes back
  (6.3).
- **Verb contract:** the same scenario files (a list of resources, a provider config, expected
  plan hash and report) are run through each SDK and through the CLI; all must give the same
  plan hash and the same report. The scenarios live in the repo once, as data.
- **End to end:** each SDK runs `plan`, `apply`, a second `plan` that shows no changes, and
  `destroy` against the fake provider through the real shared library, then checks the ledger
  and the fake's call log.
- **Generation:** the "hand-written part does not grow" check: adding a test-only message to a
  fixture proto changes generated files only.
- **Coverage:** the 99% line and branch floor applies to the hand-written SDK source in each
  language, by that language's coverage tool, wired into `cargo xtask coverage`. The tool choice
  is made in each SDK's first PR.
- Static types: the generated code must pass the language's type checker in strict mode.

### 10.5 Real clouds

Each provider: `kit::run_all` and the crash sweep on its emulator in pull requests. After merge
and only on demand, behind an explicit live flag that refuses to run without a named account and
a spend cap: the kit against the real service, recordings scrubbed before they are committed.
Live tests never run in pull-request CI.

### 10.6 What runs where

| Suite | Pull request | After merge / on demand |
|---|---|---|
| fmt, clippy, deny, SPDX, run-lines, dependency direction, public lint, sign-off (`cargo xtask check`) | yes | yes |
| unit, golden, engine, CLI, kit on fake and emulators; 99% line and branch per crate (`cargo xtask coverage`) | yes | yes |
| ABI conformance in-process; symbol table check | yes | yes |
| SDK conformance on Linux x86-64 for each SDK (`cargo xtask sdk-test`) | yes (proposal) | all supported platforms |
| built `cdylib` loaded dynamically, Miri, sanitizer builds | no | yes |
| macOS arm64 and Linux arm64 builds and SDK conformance | no | yes |
| live cloud runs | never | explicit, on demand |

The current workflow has two jobs (`check`, `coverage`); the SDK job would be a third, one
`cargo xtask` verb per job as the existing rule requires.

## 11. Open questions

These are the decisions this document cannot make alone:

1. **Loadable providers** (3.5, 8.3): is there a provider that must be written outside Rust? The
   recommendation is no, so providers are linked into the core.
2. **SDK-language tests** (10.4): `CONTRIBUTING.md` says tests are Rust. The recommendation is a
   scoped exception for `sdk/<language>`, run through one xtask verb.
3. **Encoding strategy** (6.3): generated encoders (recommended) or each language's protobuf
   runtime.
4. **The first real cloud's order** after the fake, and the live-account and spend rules for
   section 10.5.
5. **Descriptor-driven lowering** (6.4) changes where kind knowledge lives; confirm it is wanted
   before more kinds are added by hand.

## 12. Implementation sequence (proposal)

Each step is one pull request, keeps `cargo xtask check` and `cargo xtask coverage` green, and
lists its tests by name. The first part ends with a working Python SDK; real clouds come after.

1. **This document.**
2. **`ucdk-api` and the `ucdk.api.v1` protos.** Session config, plan, report, status, findings,
   error; conversions to and from the engine's values; the outcome-to-exit table; the CLI is
   moved onto it with its exit codes and plan-text goldens unchanged.
3. **File ledger** (`--state` for the CLI), so a run can be resumed across processes before the
   library exists. Includes the ledger conformance suite.
4. **Descriptor-driven lowering** for `service` and `job`, with the existing plan hashes and
   goldens unchanged as the proof; the kind-specific `match` in `catalog` is gone.
5. **Read-only status in the engine** (`Request::Status`) and the CLI `status` verb.
6. **`ucdk-capi`: the `cdylib` and the generated C header.** The per-crate `unsafe` exemption and
   its check in `cargo xtask check`; the eight functions; panic containment; ABI conformance
   tests (10.3); symbol-table check; version rules.
7. **`ucdk-codegen` and `cargo xtask gen-sdk`.** Descriptor reading, a fixture proto with golden
   output, the Python target.
8. **Python SDK** (`sdk/python`): loader, `Session`, generated templates, platform wheel layout;
   `cargo xtask sdk-test`; the wire contract and verb contract tests. **Milestone: plan, apply,
   destroy through Python against the fake provider, end to end, through the real library.**
9. **CONTRIBUTING and CI updates** for SDK tests and the third CI job (the amendment in 10.4).
10. **TypeScript target and SDK** (`sdk/typescript`): generator target, binding decision, worker
    thread wrapper, same contract tests and the same scenario files.
11. **Lease and fencing** with an injected clock (9, item 2), with the crash sweep extended.
12. **Leak sweep**: a read-only comparison of the cell's stamped objects with the ledger, as a
    new verb in the engine, `ucdk-api`, the library (a minor ABI bump) and both SDKs.
13. **First real cloud, emulator first.** Provider crate, emulator crate, adapter crate;
    `kit::run_all` and the crash sweep on the emulator; the registry entry. Then the
    object-store ledger on that cloud's conditional writes (9, item 3).
14. **Second and third clouds, and on-premises**, each as step 13, in the order decided in
    question 4. On-premises uses the same trait against infrastructure the operator names and object
    store.
15. **Release packaging**: per-platform library builds, wheels and npm packages, the version
    policy of section 5 enforced by `cargo xtask check`.

A step may be split further in review; the order is the proposal. Steps 2 to 5 do not depend on
the C ABI and each improve the CLI on their own, which keeps them useful even if the shape in
section 3 changes.
