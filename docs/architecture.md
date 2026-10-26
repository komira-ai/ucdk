# ucdk architecture

Status: design. This document says what exists in the repository today, and proposes how the
framework around it is laid out before more clouds and resources are added. Every section marks
which it is: **Exists** means the code is on `main`; **Proposal** means it is a decision to be
made in review and has no code yet. Where a claim about the code could rot, it names the crate
so a reader can check it.

## 1. The model in one page

- **One core**, written in Rust, built once as a shared library (`libucdk_capi.so` on Linux,
  `libucdk_capi.dylib` on macOS) with a small, stable C ABI.
- **Thin SDKs** in any language (Python and TypeScript first) load that library and call it.
  An SDK holds no planning, ordering, ledger or provider logic.
- **Resources are protobuf messages**: the `ucdk.resource.v1` protos
  (`crates/ucdk-proto/proto/ucdk/resource/v1/resource.proto`). Everything that crosses the
  library boundary is protobuf bytes.
- **Each SDK is generated from those protos.** Adding a resource to a cloud means adding a proto
  message and a provider implementation; no SDK source is written by hand for it, and neither is
  any lowering code (see 6.4).
- **One state per environment.** The ledger of what a run owns is stored in the environment it
  deploys into, not on the machine that runs the tool.

```
  Python SDK     TypeScript SDK     (other SDKs)          ucdk CLI
       \               |               /                      |
        \              |              /                       |
         +---- C ABI (libucdk_capi, protobuf bytes) ----+          |
                       |                                      |
                  ucdk-capi  (the only crate with unsafe code)
                       |                                      |
                  ucdk-api   (protobuf <-> engine values, one exit-code table)
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
| Ledger | `ucdk-ledger` | records, the single-document compare-and-swap `Ledger` trait, `MemoryLedger`, `FileLedger`, and the lease with its fencing token (`acquire`, `renew`, `release`, `write_fenced`) on both | no |
| Engine | `ucdk-engine` | `Engine::handle(Request) -> Outcome`: write-ahead apply, re-observe before each mutation, closed-world delete, opt-in rollback, outputs, status. Synchronous | no |
| Host: CLI | `ucdk-cli` | `plan`, `apply`, `destroy` against the fake, as a library (`run(args, &mut World) -> u8`) with a three-line `main.rs` | no |
| Repository checks | `xtask` | `cargo xtask check`, `coverage`, `fetch` | no |
| API | `ucdk-api` | the `ucdk.api.v1` protobuf messages for everything that is not a resource (session config, requests, plan, report, findings, error; `StatusRequest` and `StatusResponse` for the engine's read-only status) and the code converting requests to the engine's requests and the engine's outcomes to messages (a plan only goes out: a caller sends the hash, never a plan); the single table from outcome to exit class that both the CLI and the SDKs use | no |
| C ABI | `ucdk-capi` | the `cdylib`, the C header (`crates/ucdk-capi/include/ucdk.h`), panic containment, buffer ownership. The only crate in the workspace allowed `unsafe` | no |
| SDKs | `sdk/python` (exists), `sdk/typescript` (exists) | the loader, the session wrapper, and generated typed templates | n/a |
| Generator | `ucdk-codegen` | compiles the protos in-process, reads the descriptors and returns the generated SDK sources as files (`cargo xtask gen-sdk` writes them; the `sdk-fresh` gate of `check` compares them with the committed ones). Python and TypeScript are the targets | no |

The engine already states the seam this document builds on: the whole surface is one pair of
plain enums, `Request` and `Outcome`, "not a wire format: a later shared library wraps exactly
this seam and nothing else" (`crates/ucdk-engine/src/lib.rs`). The C ABI is that wrapper. It adds
no behaviour; any behaviour an SDK user can reach is one the CLI test suite can reach.

## 3. The C ABI (exists: `ucdk-capi`)

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
int32_t ucdk_leaks       (ucdk_session *s, const uint8_t *req, size_t req_len, ucdk_buf *out);

/* Memory: every ucdk_buf the library fills is released here, once. Zeroed buf is a no-op. */
void    ucdk_buf_free(ucdk_buf buf);
```

That is ten symbols. The six verbs share one signature on purpose: a binding is written once.

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
  (`Ready`, `Converging`, `Failed` as `ucdk_provider::Status`, or absent), the ledger record's state,
  its recorded outputs and whether the ledger and the cloud agree (`Standing`). Read-only: it is
  `Request::Status`, which calls `Ledger::read` and `Provider::observe` and nothing else. It lists
  every recorded object and every wanted node of the list.

- `ucdk_leaks`: request is a `LeaksRequest` (empty: it is about the cell, not about a file);
  response is an `Outcome` whose `leaks` arm lists the objects that carry the cell's stamp and
  that no ledger record accounts for. Read-only: `Request::Leaks` reads the ledger, calls
  `Provider::list_stamped`, reads the ledger again, and does nothing else. It takes no lease, so a
  run may be writing; an object is accounted for if the record of either read names it, which
  keeps a concurrent run's new object (its intent is recorded before the object exists) and one it
  just deleted out of the report. A record names an object by its physical id; an intent with no
  physical id stands for one object stamped for its node (the lowest physical id), so a second
  stamped object for that node is reported. Its exit class is done when the list is empty and
  `leaks` (8) when it is not. A stamp is a claim, never an authority: nothing is deleted on it,
  and an object whose stamp was stripped is not listed by the provider and so is not reported. An
  apply ends with the same sweep and carries its leaks in its report (`Report.leaks`); it is still
  done. Added in ABI minor 2.

A later verb is a new symbol and a new minor version; nothing existing changes.

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

### 3.6 As built, and what is left

`crates/ucdk-capi` builds as `cdylib` and `rlib` (the tests call the same functions in-process).
Where the code is more specific than the text above:

- **Ten functions.** `abi_version`, `session_open`, `session_close`, the six verbs
  and `buf_free`. The header is hand-maintained; `tests/header.rs` compares it with the exports
  (names, order, signatures, constants and the buffer layout), and checks that each name
  resolves, by a linker declaration, to the function of that name.
- **`ucdk_load_catalog`** takes a `ResourceList` and answers `ucdk.api.v1.CatalogResponse` (the
  lowered nodes in dependency order with kind, wanted flag and dependencies, or the findings
  that refuse the list). The other four verbs answer `ucdk.api.v1.Outcome`.
- **Sessions.** `SessionConfig.provider` is `fake`, `fake-limited` or `fake-leaky` (the fake with one stamped object no record names, for hosts that test the leak sweep); `ledger_kind` is empty or
  `memory`, or `file` with `ledger_location`; `cell` is empty (a local cell) or `machine/cell/uid`;
  `provider_options` must be empty and `credentials` a known value. Anything else the library
  lacks is `UCDK_ABI`. A ledger that cannot be opened (held by another session, a missing
  directory) is `UCDK_INTERNAL`.
- **Handles are numbers.** A `ucdk_session *` carries a number from a counter that only grows,
  and the library keeps the map from open numbers to sessions. A closed, twice-closed or
  never-opened handle is refused (`UCDK_INVALID_ARGUMENT`), and stays refused when the allocator
  reuses an address or a later session opens; nothing reads through the pointer. A close during a
  call is safe: the call keeps its session alive and, if it holds the last count, drops it inside
  its `catch_unwind`.
- **Busy and poisoned.** A call takes the session's lock without waiting (`UCDK_BUSY`). A panic
  inside a call leaves the lock poisoned, and every later call on that session is `UCDK_PANIC`
  until it is closed.
- **Freeing a buffer twice is undefined**, as 4.1 says; it is not detected. A zeroed buffer, or one
  with a zero length, is ignored.
- **Not done.** The library does not check an SDK's version: the loader rule of section 5 is the
  SDK's (the Python loader does it, `sdk/python/ucdk/_library.py`). The `UCDK_ABI` refusal of a different major (10.3) therefore has no library side yet. A
  version script is not used (a Rust `cdylib` exports only its `#[no_mangle]` functions, but the
  built library's symbol table is not read). The built `cdylib` loaded dynamically, Miri and the
  sanitizer builds of 10.3 are not run: they are after-merge suites, outside the pull-request
  budget.

## 4. Memory ownership and error model across the boundary (exists)

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
with no effect, 6 failed after changing something, 8 leaks found by the leak sweep): `ucdk-api` holds the one function from
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

## 5. ABI versioning and compatibility (partly exists)

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

## 6. How SDKs are generated (exists for Python and TypeScript: `ucdk-codegen`)

### 6.1 Input

The descriptor set for `ucdk.resource.v1` and `ucdk.api.v1`, produced in-process by `protox` with
source info (comments) included. This is the same compile `ucdk-proto/build.rs` already does, so
no `protoc` and no per-language protobuf toolchain is needed to generate anything.

### 6.2 What is generated, and what is hand-written

| Generated, never edited | Hand-written, small and fixed |
|---|---|
| message classes and enums for both packages, with docs taken from the proto comments | the C ABI binding: load the library, check the version, call the ten functions |
| binary encode and decode for those messages | the `Session` type: `open`, `load_catalog`, `plan`, `apply`, `destroy`, `status`, `close`, and the error type built from the status codes |
| one typed template per resource kind (the fields of the `Resource.body` oneof): required fields as constructor arguments, optional ones as keywords, oneofs as separate named constructors (an image is `Image.digest(...)` or `Image.step_output(...)`, never a free-form dict) | locating the library (6.5) |
| typed output references from the `exposes` option: `service.url` is a `Ref`, not a string | the worker thread and the promises (TypeScript) |
| the portability marker per kind (`PORTABLE` or `CLOUD_BOUND`) as metadata | packaging metadata |

The hand-written part does not grow when a resource is added. A test enforces that (10.4).

### 6.3 Encoding: generated, no protobuf runtime dependency (decided, built)

The generator emits the encoder and decoder itself (option B of the design): an SDK has no
protobuf runtime dependency, no per-language protobuf generator and no version skew. The subset
of proto3 it writes code for is the one the protos use: strings, booleans, `int32`, `int64`,
`uint32`, `uint64`, enums, messages, `repeated` strings and messages, `map<string, T>`, `oneof`
and `optional`; any other field type is refused by name when the generator reads it (a new proto
feature is a decision in `ucdk-codegen/src/model.rs`, not a field an SDK silently drops). What
the targets read from the descriptors (the field types, the order a message is written in, the
resource kinds) is shared between them in `model.rs` and `kinds.rs`; what they write is not.
TypeScript writes 32 bit integers as `number`s and 64 bit ones as `bigint`s, an enum as a constant
object and its type, fields in camel case, and only syntax that is nothing but types, so that Node
can run the files by stripping them.

The bytes are the ones the Rust side writes, not merely valid protobuf: a message is written in
field-number order except that a whole oneof sits where its lowest arm is (that is how prost
orders its fields), a singular scalar is left out when it is the default, an `optional` field and
a message when it is absent, a map in key order with a default key or value left out of an entry.

The contract test is `sdk/wire-golden`: for each case, a golden file of the bytes the Rust
messages encode to (written and held by `crates/ucdk-api/tests/sdk_golden.rs`, one case per
message shape, with `manifest.txt` naming the message of each). Every SDK builds the same value by
hand, must encode it to the same bytes and must decode the bytes to the same value
(`sdk/python/tests/test_wire_golden.py`, `sdk/typescript/tests/wire_golden.test.ts`). `crates/ucdk-codegen/tests/golden_coverage.rs` fails
unless the goldens set every field of every message of both packages.

### 6.4 What "adding a resource" costs

A proto message with options, and a provider that hosts the kind. Nothing else on the planning
side. Lowering is driven by the descriptors (`ucdk-plan/src/catalog/descriptor.rs`, built once
into a `Catalog` from the resource list message, which `ucdk-proto` embeds as `DESCRIPTOR_SET`):

- The kinds are the arms of the `body` oneof of the resource message. An arm's field name is the
  kind (`service`, `job`) and its message is the type.
- Message options: `portability` (required; `UNSET` is refused when the catalog is read) and
  `exposes` (the outputs a `Ref` may ask for).
- A field's **shape**, its protobuf type, decides how it lowers, so no option says "this is a
  reference" or "this is a secret": `Image` is the checked image, `repeated string` is one node
  field per element (`<name>.<index>`), `map<string, Value>` is one node field per key or a
  reference to another resource's output, `map<string, SecretRef>` is one node field per key
  holding `[store/]name[@version]` (and a key also in a value map of the same message is refused),
  `string`, `uint32`, `int32` and `int64` are one field, and any other message is one text field
  built from a template. A field of any other type is refused when the catalog is read, not
  skipped. So are options that cannot apply: `render` off a message field, `role` on a map or
  together with `unlowered` or `node_name`, an integer `fallback` outside its field's type, a role
  name that is not one lower-case word of letters and digits (or is `run`), and two roles, or two
  node field names (empty, containing `.`, or equal), that collide.
- Field options say only what the type cannot: `node_name` (the node field name when it is not the
  field's), `render` (the template of a message-typed field, `{cpu_millis}m/{memory_mb}MB`),
  `fallback` (the versioned default written where the author wrote nothing), `role` (on a
  message-typed arm: a second node `<resource>/<role>`, always emitted, wanted when the arm is
  written, with the arm message's scalar fields as its fields) and `unlowered` (a choice that
  changes no node field). A message field that is not written and has no `fallback` renders
  its template with every placeholder as zero or empty. `service` and `job` use exactly these; their defaults and role names are
  in `resource.proto`.

The shapes themselves (`Image`, `Value`, `SecretRef`, `Uses`, and the rendering of scalars) are
code, because they carry semantics a template cannot (the platform check, the output
reference, the secret text). That code is per shape and shared by every kind; adding a kind does
not touch it, and adding a shape (a new `Ref` field, say) is a change to the planner that every
kind then gets. The proof of "no code per kind" is `tests/descriptor_lowering.rs`: a `Queue` type
declared only in `tests/proto/queue.proto` validates, lowers, plans, hashes and exposes `HOST`,
while the existing plan-hash goldens and every other plan test run unchanged against the built-in
catalog, which is read from `resource.proto` the same way.

Not covered, and said so: a kind whose lowering needs a new shape (a bare `Ref` field that is a
dependency and not a value, a repeated message, a bool) is refused by `Catalog::new` until the
shape is added; and a provider still has to be written for the kind.

### 6.5 How an SDK finds and loads the library

- The library ships **inside the SDK package**, one per platform: a platform wheel for Python and
  an optional per-platform package for TypeScript, covering Linux x86-64, Linux arm64 and macOS
  arm64. The loader looks in the package directory first.
- An explicit `library_path` argument to the constructor overrides it (for tests and for a
  vendored build). There is no environment-variable lookup: configuration is arguments. (Built for
  Python: `Library(library_path)` and `Session(config, library_path=...)`; without a path the
  package's `_lib` directory is searched. The wheel layout itself is step 15.)
- Python binds with `ctypes` (no compiled extension, so one pure-Python wheel plus a data file per
  platform). TypeScript binds with `node:ffi`, the FFI built into Node 26 (decided and built: no
  native addon and no package, so the npm package is TypeScript source plus the library). It is
  experimental in Node (a warning is printed once), so the SDK names the Node it needs
  (`engines`, 26.12.1 or newer) and the tests run on a Node that `xtask` pins by version and
  SHA-256. `node:ffi` has no struct types, so the one function that takes a struct by value,
  `ucdk_buf_free(ucdk_buf)`, is declared as the two integer arguments the struct is passed as
  on Linux x86-64 and on arm64 (a pointer and a `size_t`); the SDK is not for a platform where
  that is not so.
- Blocking calls: the Python SDK releases the GIL by construction under `ctypes`; the TypeScript
  SDK (built) loads the library on a worker thread that belongs to the session, runs each call
  there one at a time in the order made, and exposes promises. The worker keeps the process alive
  only while a call is waiting for it.

### 6.6 Generated code in the repository (built)

Generated SDK sources are **committed**, under `sdk/python/ucdk/generated/` and
`sdk/typescript/src/generated/`. `cargo xtask gen-sdk`
writes them (and removes a generated file the generator no longer writes); the `sdk-fresh` gate
of `cargo xtask check` regenerates them in memory and fails on a file that differs, is missing, or
is extra, so a proto or generator change shows in review as the diff of the SDK it changes, and no
checkout ever builds an SDK from a stale tree. The repository's rule that nothing generated is
committed still holds for the Rust types (`ucdk-proto/build.rs` writes them into the build
directory); the reason it does not for an SDK is that a Python package and TypeScript source are
used from a checkout without a build step.

The generator's own output for a fixed proto (`crates/ucdk-codegen/tests/proto`, every supported
field shape, comments, names that need care) is kept as golden files in
`crates/ucdk-codegen/tests/golden`, held by a Rust test and loaded by the Python and the TypeScript tests (the files of both
targets are in one directory, since their paths differ).

## 7. Resources as protos (exists)

`ucdk.resource.v1` defines `Resource { id, uses, oneof body { Service, Job } }`, `ResourceList`,
`Ref` (a resource and one of its outputs, standard or named), `Value` (literal, parameter or
ref), `Image` (a step output or a digest, plus platform), `SecretRef`, and the message options
`portability` and `exposes` and the field options of 6.4. The lowering, read from those, produces, for each resource, a `run` node and one extra
node of the type's other role. Plan hashes are computed over the lowered nodes, so a change to
lowering is a change to hashes and is gated by goldens.

A kind that is only available on one cloud is `CLOUD_BOUND`; the portable ones are mapped by each
provider. A provider reports which kinds it hosts (`Provider::hosts`); the engine refuses a plan,
apply or status whose list wants a kind the chosen provider does not host, before any provider
call or ledger write (`Refusal::UnhostedKinds`, `ucdk.api.v1.UnhostedKinds`, exit class refused).
The command line prints it as `refused: kind job is not hosted by fake-limited (nightly/run)` with
exit code 3, and the C ABI and the SDKs get it as a response field. A destroy is not refused for
it: it wants nothing.

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
and fails with `LedgerError::Stale` if another writer got there first. `MemoryLedger` keeps the
document in the process, for tests. `FileLedger` keeps it in a file, so a run can be resumed by
another process on the same machine (below). Both keep a lease in the document (9.2): an apply
or a destroy takes it for its run, and every ledger write it makes carries the lease's fence.

**Decided direction:** deployment state is stored in each environment. The ledger for a cell
lives in storage the provider owns inside that environment (an object store in the same account,
project or subscription; for on-premises, a store the operator names), so that whoever can reach
the environment with the right credentials sees the same state, and no machine-local file is the
source of truth.

**Proposals, in order:**

1. **Done: the file ledger** (`FileLedger`; single machine, not a team backend), so a run can
   resume after a crash before any cloud exists. See 9.1.
2. **Done: the lease** with an injected clock and fencing in the ledger write, so a stalled run
   cannot write after a newer one took over. Fencing protects the ledger, not the cloud. See 9.2.
3. An object-store ledger on conditional writes (create-only, then if-match), with the lease in
   the same document, implemented by the provider that owns the environment. A provider that
   cannot give conditional writes is refused as a ledger home.

### 9.1 The file ledger

`FileLedger::open(path)` keeps the whole document in one file.

- **Writes.** Each write builds the next document, writes it to `<path>.tmp` in the same directory
  (mode 0600), flushes the file, renames it over `<path>`, and flushes the directory. A reader
  sees the old document or the new one. A crash before the rename leaves the old document and a
  stale `.tmp`, which the next open removes. If a write fails, the handle re-reads the file (it
  may have landed) so the next compare-and-swap is made against what is there.
- **Format.** Line-oriented text: a header `ucdk-ledger <major>.<minor>`, the document version
  (the `Token`), one block per record, and an `end <records> <checksum>` line. Values are
  percent-escaped. The minor only adds tags: a build reads a document of its major and any
  minor and ignores tags it does not know, but refuses to rewrite a document with a newer minor
  (it would drop them). A newer major is refused on open by name. Nothing is guessed.
- **Refusals.** A file that is empty, cut short at any length, changed by any one byte, not a
  ledger, or whole but invalid (missing field, unknown state, duplicate record, wrong count) is
  refused on open with the line and the reason (`OpenError::Corrupt`). The file is never
  repaired or overwritten by a refused open. The checksum guards against accidents, not
  tampering.
- **Several handles at once.** Any number of handles, in this process or in others, may be open
  on one path. Each call reads the file afresh; each change takes an exclusive advisory lock on
  `<path>.lock` for its read-modify-write alone and gives it back before it returns, so two
  changes never interleave and no write is lost. The lock is never held between calls, so it
  keeps no live process out: the lease (9.2) is what arbitrates between two live runs. The lock
  does not cross network file systems reliably and does not stop a process that ignores it.
- **The lease in the file.** Format 1.1 adds two lines before the first record: `fence <n>`, the
  fence of the last grant, and `lease <holder> <expires-ms>` while one is held. A lease change
  rewrites the file like a write does, and does not move the document's version.
- **Not yet wired to the CLI.** The CLI's only cloud is the in-memory fake, so a state file
  could not resume anything; `--state` arrives with the first provider whose objects outlive
  the process. to the run that asks and never written.
`SessionConfig` names the ledger by kind and location, plus a credential source by name.

### 9.2 The lease and the fence

A run that writes takes the ledger's lease first (`Ledger::acquire(holder, ttl)`) and gets a
`Lease`: its `holder` (a name for the other side's message), a `Fence` and the time it lapses.
Every write it makes is `Ledger::write_fenced(fence, token, change)`, and the ledger refuses it
unless the fence is the one it granted last and the lease has not lapsed.

- **The fence only goes up.** Each grant is the last one plus one, whether the earlier lease was
  released or lapsed, and the number is kept in the document through a release. A holder that
  stalled past its lease, whose process is still running, is refused at its next write with
  `LedgerError::Fenced`, which the file ledger's advisory lock cannot do: the lock is held only
  while one change is made, the fence is kept in the document.
- **Renew, release.** `renew(fence, ttl)` moves the lapse to `now + ttl` for the current fence
  of a lease that has not lapsed; a lapsed one is acquired again (a new fence), never revived.
  `release(fence)` lets the next `acquire` in at once. The holder the lease names takes it again
  at once, live or lapsed, with a new fence (the old one is fenced out): a run whose release was
  lost is not made to wait out its own lease. Holder names are therefore identities: two live
  runs given one name do not exclude each other, though the fence still refuses the older.
- **Time is the ledger's.** A ledger has a `Clock` (`SystemClock`, or a `ManualClock` a test
  moves), and a holder's own clock decides nothing. A new holder is granted the lease only once
  the clock reads the lapse plus `MAX_CLOCK_SKEW` (5 seconds); the lapse itself already refuses
  the old holder's writes. The gap is the bound on how far two handles' clocks on one document
  may disagree. Skew beyond it, or a clock stepped back, is not covered; the fence still refuses
  a holder that was overtaken.
- **Leasing does not move the token.** A grant, a renewal and a release leave the document's
  version alone, so a plan made before a run took the lease is still the plan the run checks.
- **Unfenced writes.** `Ledger::write` is for a document nobody leases: it is refused as
  `LedgerError::Leased` while a lease is live (or lapsed less than the skew bound ago).
- **The engine.** An apply or a destroy acquires after its refusals that need no ledger and
  before it observes anything, renews at the start of every step, and releases when it ends,
  whatever it ended with. When taking the lease fails with an answer that does not say whether
  the grant landed, or giving it back fails after a run that stopped, the run is
  `EngineError::LeaseUnconfirmed` (wire code `ERROR_CODE_LEDGER`), exit class failed-changed
  (6), never "safe to run again": the same holder reruns at once, any other is refused until the
  lease lapses. A run that finished stands when its release fails. A plan or a status takes no lease. The holder defaults to a new name
  per run (`Engine::with_holder` names it) and the lease lasts ten minutes between renewals
  (`Engine::with_lease_ttl`). A run refused for the lease is `Refusal::LeaseHeld` (holder and the
  longest wait), `ucdk.api.v1.LeaseHeld`, exit class refused, nothing read or written. A run
  whose lease is lost stops with `EngineError::LeaseLost` (`ERROR_CODE_LEASE_LOST`), does not roll
  back, and what it had begun is resolved by the next holder from the intent record and the stamp.
- **Naming the holder.** `SessionConfig.holder` (both SDKs) and `ucdk apply|destroy --holder`
  name the holder; unnamed, every run takes a new name (process id and a random number, so equal
  process ids in two containers do not collide). Name the holder a run left a lease under to take
  it again at once.
- **Not yet.** The lease length is not in `SessionConfig`, and no verb reads or breaks a lease by
  hand.

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
| `ucdk-plan` | validation findings, lowering, ordering, plan and its hash | unit and golden tests (`tests/catalog.rs`, `descriptor_lowering.rs`, `graph.rs`, `plan.rs`, `hash_text.rs`) | a changed hash, a missed cycle or dangling reference, a foreign object not refused |
| providers | the trait contract | `kit::run_all` on each provider; fake: fault injection (`fail_at_call`, `read_lag`, `ambiguous_success`, `foreign`, `race_next`, `create_failed`, `converging_reads`) | create that does not stamp atomically, an ambiguous result treated as failure, a delete of a foreign object |
| `ucdk-ledger` | CAS semantics, record encoding | unit tests; a conformance suite (`tests/conformance.rs`) run on both implementations; file-format and crash tests (`tests/file.rs`) | lost update, stale write accepted, a torn or altered file read as a ledger |
| `ucdk-engine` | apply, destroy, resume, rollback, outputs, status | integration tests over the fake with call logs (`tests/apply.rs`, `crash.rs`, `delete.rs`, `outputs.rs`, `status.rs`, `read_status.rs`); the crash sweeps and rollback scenarios run on the memory and the file ledger | a mutation before its intent is recorded, a record dropped after a failed read, a delete of an object that is not ours |
| `ucdk-cli` | arguments, exit codes, plan text, end to end | in-process runs over a `World` (`tests/cli.rs`, `e2e.rs`, `status.rs`, `status_verb.rs`) with plan-text goldens | wrong exit class, a drifting plan text |
| `ucdk-api` (proposal) | message-to-value conversion in both directions; the outcome-to-exit table | round-trip tests; an exhaustive match so a new `Outcome` or `Refusal` variant fails to compile until it is mapped | a field dropped in conversion, an unmapped outcome |
| `ucdk-capi` | the ten functions, called as C would | see 10.3 | ownership, panic, version and malformed-input defects |
| SDKs | binding and generated code | see 10.4 | an SDK that disagrees with the core or with another SDK |
| generator | output for fixture protos; the committed SDK; a kind added to `resource.proto` | golden output (`tests/generate.rs`, `templates.rs`, `golden_coverage.rs`), the `sdk-fresh` gate, and the fixture's golden loaded and run by the Python and TypeScript tests | generated code that does not load or encode wrongly, a refused-in-silence field type, a stale committed SDK |

### 10.2 Crash and fault injection

The engine's central claim is that a crash at any point leaves a state the next run resolves by
looking. It is tested by running a second `run()` over the same world after stopping the first at
call N, for every N of a plan, against each fault the fake can inject, and asserting: at most one
object per node, no record dropped without a read showing the object gone, no delete of an object
without both stamp and record. The same sweep is run again for each real provider's emulator
(8.2). With the lease (exists), the sweep is run again with a holder that dies where the first run
stopped and never releases: the next run is refused while the lease holds, then the injected
clock is moved past the lease and the skew bound and the run resumes to the clean state; and a
run whose clock is moved past its lease mid-run, with a second run finishing the work, is
refused by the ledger at its next write and changes nothing more.

### 10.3 ABI conformance (in-process part exists)

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

### 10.4 SDK conformance (Python and TypeScript exist; the scenario files are shared with the CLI)

The SDK tests are written in the SDK's language, because the point is to exercise the binding.
This is an exception to "tests are Rust" in `CONTRIBUTING.md`, scoped to `sdk/<language>` and
driven only through the `cargo xtask sdk-test` verb so the workflow still runs one verb per line.
The Python SDK's tests are the standard library's `unittest` only, the TypeScript SDK's are
Node's built-in `node:test` only: nothing is installed and no network is used (the pinned Node
is fetched and verified by `xtask`, as the other tools are).

- **Wire contract:** every golden message encodes to the same bytes in every SDK and decodes back
  (6.3).
- **Verb contract:** the same scenario is run through each SDK and through the CLI; all must give
  the same plan hash, steps, report and status. As built, the scenario's data is what the CLI prints
  for it (`crates/ucdk-cli/tests/data`, checked by `crates/ucdk-cli/tests/scenario.rs`), and
  `sdk/python/tests/test_scenarios.py` and `sdk/typescript/tests/scenarios.test.ts` read the same
  files, so there is one set of expected
  values.
- **End to end:** each SDK runs `plan`, `apply`, a second `plan` that shows no changes, and
  `destroy` against the fake provider through the real shared library (`cargo xtask sdk-test`
  builds `ucdk-capi` first), and checks the refusals come back as data. Both SDKs' tests also hold
  the loader (a wrong major, an older minor, a file that is not the library), a closed session
  (refused by the SDK, and a closed handle refused by the library) and every status code's
  error; Python's also holds the search of the package directory, which the TypeScript SDK does
  not have yet (it takes `libraryPath` only, until the package layout of step 15).
- **Generation:** the "hand-written part does not grow" check: adding a test-only message to a
  fixture proto changes generated files only.
- **Coverage:** the 99% line and branch floor applies to the hand-written SDK source in each
  language, by that language's coverage tool, wired into `cargo xtask coverage`. **Not wired for
  Python or TypeScript yet**: the standard library's `trace` measures lines only (the hand-written
  Python, 215 executable lines, is fully executed by the tests when measured by hand). Node's test
  runner measures lines and branches with nothing to install (`node --test
  --experimental-test-coverage`, threshold flags included, and it reads the worker thread too);
  by hand the hand-written TypeScript (`src`, without `src/generated`) is at 99.0% of its lines
  and 96.8% of its branches. What is not run is the path of a worker thread that ends with calls
  waiting, a defence against a crash that no test can cause without a seam in the SDK, and the
  refusal of a library of another ABI as the worker reports it (the SDK's ABI is a constant, and
  the loader's own tests lower and raise it). Wiring a gate, and deciding whether to add that
  seam, is open.
- Static types: the generated code must pass the language's type checker in strict mode.
  **Not run yet** for TypeScript: `tsc` is a package, and CI installs none. The sources are
  written for `strict`, `erasableSyntaxOnly` and `verbatimModuleSyntax` (`sdk/typescript/tsconfig.json`)
  and were checked with `tsc` by hand once; a gate waits for a way to get the compiler that
  respects the no-package rule.

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
| SDK conformance on Linux x86-64 for each SDK (`cargo xtask sdk-test`) | yes | all supported platforms |
| built `cdylib` loaded dynamically, Miri, sanitizer builds | no | yes |
| macOS arm64 and Linux arm64 builds and SDK conformance | no | yes |
| live cloud runs | never | explicit, on demand |

The workflow has three jobs (`check`, `coverage`, `sdk`), one `cargo xtask` verb per job as the
rule requires.

## 11. Open questions

These are the decisions this document cannot make alone:

1. **Loadable providers** (3.5, 8.3): is there a provider that must be written outside Rust? The
   recommendation is no, so providers are linked into the core.
2. **SDK-language tests** (10.4): decided: a scoped exception for `sdk/<language>`, run through
   `cargo xtask sdk-test` (`CONTRIBUTING.md`).
3. **Encoding strategy** (6.3): decided: generated encoders.
4. **The first real cloud's order** after the fake, and the live-account and spend rules for
   section 10.5.
5. **Descriptor-driven lowering** (6.4) is built; kinds are added as a message with options.

## 12. Implementation sequence (proposal)

Each step is one pull request, keeps `cargo xtask check` and `cargo xtask coverage` green, and
lists its tests by name. The first part ends with working Python and TypeScript SDKs; real clouds come after.

1. **This document.**
2. **`ucdk-api` and the `ucdk.api.v1` protos.** Session config, plan, report, status, findings,
   error; conversions to and from the engine's values; the outcome-to-exit table; the CLI is
   moved onto it with its exit codes and plan-text goldens unchanged.
3. **File ledger** (done: `FileLedger`, the ledger conformance suite, the engine's crash and rollback
   scenarios run on both ledgers). The CLI `--state` flag moves to the first provider whose
   objects outlive the process (9.1).
4. **Descriptor-driven lowering** (done: `Catalog`, read from the descriptors; the existing plan
   hashes and goldens pass unchanged; the kind-specific `match` in `catalog` is gone, and
   `tests/descriptor_lowering.rs` lowers a kind declared only in a test proto).
5. **Read-only status in the engine** (done: `Request::Status` reads the ledger and observes each recorded and each wanted object, with no ledger write and no provider mutation; `ucdk.api.v1` `StatusRequest` and `StatusResponse`, exit class done or failed with no effect; `ucdk status --provider --input`).
6. **`ucdk-capi`: the `cdylib` and the C header** (done: `crates/ucdk-capi`; the per-crate `unsafe`
   exemption and its `unsafe-scope` gate in `cargo xtask check`; the nine functions; panic
   containment; the in-process ABI conformance tests of 10.3; a test that the checked-in header
   matches the exports; see 3.6 for what is not done).
7. **`ucdk-codegen` and `cargo xtask gen-sdk`** (done: `crates/ucdk-codegen`, the Python target
   (the TypeScript target of step 10 came after, on the descriptor reading both share):
   message classes with their own encoders and decoders, typed resource templates and output
   references read from the options, `sdk/python/ucdk/generated` committed and held by the
   `sdk-fresh` gate; a fixture proto with golden output; the wire goldens in `sdk/wire-golden`,
   written by `ucdk-api/tests/sdk_golden.rs` and checked by `sdk/python/tests`; the scoped
   Python-test exception, `cargo xtask sdk-test` and the third CI job of step 9 came with it).
8. **Python SDK** (`sdk/python`) (done: the loader with the version rule of section 5, `Session`,
   the errors, the generated templates; `cargo xtask sdk-test` builds the library and runs the
   wire contract and verb contract tests, which read the CLI's scenario data). **Milestone: plan,
   apply, destroy through Python against the fake provider, end to end, through the real library.**
   Not done: the platform wheel layout (step 15) and a Python coverage gate (10.4).
9. **CONTRIBUTING and CI updates** for SDK tests and the third CI job (done with step 7: the
   scoped exception in `CONTRIBUTING.md`, `cargo xtask sdk-test`, the `sdk` job).
10. **TypeScript target and SDK** (`sdk/typescript`) (done: the generator target (messages with
    their own encoders, typed templates and output references, `sdk/typescript/src/generated`
    committed and held by the `sdk-fresh` gate); the binding with `node:ffi` and the ABI rule of
    section 5, `Session` on a worker thread with promises, one error class per status code;
    `cargo xtask sdk-test` runs the `node:test` tests on a Node it pins and verifies, and they hold
    the same wire goldens, scenario files and refusals as Python's). **Milestone: plan, apply,
    destroy through TypeScript against the fake provider, end to end, through the real library.**
    Not done: the npm package layout (step 15), a coverage gate and a type-check gate (10.4).
11. **Lease and fencing** with an injected clock (done: 9.2; the fence in every engine write, `Refusal::LeaseHeld` and `LEASE_LOST` through `ucdk-api`, the CLI, the library and both SDKs, the crash sweep run again with a dead holder).
12. **Leak sweep** (done: `Request::Leaks`, a read-only comparison of the cell's stamped objects
    with the ledger; `LeaksRequest` and `LeaksResponse` in `ucdk-api` with exit class `leaks` (8),
    `ucdk leaks`, `ucdk_leaks` in the library (ABI minor 2) and both SDKs; an apply ends with the
    sweep and reports leaks without failing; a stamp alone never authorises a delete).
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
