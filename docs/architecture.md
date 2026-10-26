# ucdk architecture

Status: design, and a description of what exists. **Exists** means the code is on `main`;
**Proposal** means a decision to be made in review that has no code yet; **Historical** means a
section describes code that was removed from the tree (the deployment side: the resource catalog,
the planner, the engine, the provider trait, its contract kit and the fake provider). That code
stays recoverable from git history, and a historical section is kept so that the reasoning stays
readable, not because the code is there. Where a claim about the code could rot, it names the
crate so a reader can check it.

## 1. The model in one page

This describes the deploy-side framework as it stands today. The direction of the project has since moved to build, test and release; see [release.md](release.md), which also lists which parts of the framework stay and which go.

- **One core**, written in Rust, built once as a shared library (`libucdk_capi.so` on Linux,
  `libucdk_capi.dylib` on macOS) with a small C ABI.
- **Thin SDKs** in any language (Python and TypeScript first) load that library and call it.
  An SDK holds no ledger logic.
- **Everything that crosses the library boundary is protobuf bytes**: the `ucdk.api.v1` messages
  (`crates/ucdk-api/proto/ucdk/api/v1/api.proto`).
- **Each SDK is generated from those protos.** Adding a message adds no SDK source written by hand.
- **One ledger with a lease and a fence.** A ledger is a single document with compare-and-swap
  writes, a lease that one holder takes at a time and a fencing token that only goes up, so a
  stalled holder cannot write after another took over. A file ledger survives the process.

```
  Python SDK     TypeScript SDK     (other SDKs)          ucdk CLI
       \               |               /                      |
        \              |              /                       |
         +---- C ABI (libucdk_capi, protobuf bytes) ----+     |
                       |                                      |
                  ucdk-capi  (the only crate with unsafe code)|
                       |                                      |
                  ucdk-api   (ucdk.api.v1, ledger show, one exit-class table)
                       |______________________________________|
                                       |
                                  ucdk-ledger   CAS document, lease, fence, lease guard
                                       |
                                  ucdk-core     record paths and keys, hashes, order, clocks
```

## 2. Layers and the crate that owns each

**Exists** unless marked. The arrows only go downward; `cargo xtask check` enforces part of this
(`xtask/src/direction.rs`: the core crate never depends on the ledger) and the "pure" rule (a
crate marked `[package.metadata.ucdk] pure = true` holds no I/O, no wall clock and no `HashMap`,
and depends only on pure workspace crates).

| Layer | Crate | Owns | Pure |
|---|---|---|---|
| Shared values | `ucdk-core` | `RecordPath` and `RecordKey` (the identity of a record: a path of validated segments, and a 128-bit key hashed from a path or from a readable name, in two domains), the `Tag` and `Locator` strings a record carries, `OwnerLabel`, `Hash256` and `Framed` (a hash over fields framed by their lengths), `order` (Kahn's algorithm, smallest ready first), and `Clock` with `ManualClock` | yes |
| Ledger | `ucdk-ledger` | records, the single-document compare-and-swap `Ledger` trait, `MemoryLedger`, `FileLedger` (also opened read-only), the lease with its fencing token (`acquire`, `renew`, `release`, `write_fenced`) on both, `Snapshot` with the fence, the lease and the ledger's own clock reading, `SystemClock`, and `LeaseGuard`, the lifecycle of one holder's lease | no |
| API | `ucdk-api` | the `ucdk.api.v1` protobuf messages (session config, ledger show, error, outcome), `ledger_show`, the one function that turns a ledger into the answer, and the table of exit classes that the CLI and the SDKs use | no |
| Host: CLI | `ucdk-cli` | `ucdk ledger show`, as a library (`run(args, &mut World) -> u8`) with a three-line `main.rs` | no |
| C ABI | `ucdk-capi` | the `cdylib`, the C header (`crates/ucdk-capi/include/ucdk.h`), panic containment, buffer ownership. The only crate in the workspace allowed `unsafe` | no |
| SDKs | `sdk/python`, `sdk/typescript` | the loader, the session wrapper, the error types | n/a |
| Generator | `ucdk-codegen` | compiles the protos in-process, reads the descriptors and returns the generated SDK sources as files (`cargo xtask gen-sdk` writes them; the `sdk-fresh` gate of `check` compares them with the committed ones). Python and TypeScript are the targets | no |
| Repository checks | `xtask` | `cargo xtask check`, `coverage`, `fetch`, `gen-sdk`, `sdk-test` | no |

The C ABI adds no behaviour: a session wraps `ucdk_api::ledger_show`, and any behaviour an SDK
user can reach is one the command line can reach through the same function.

## 3. The C ABI (exists: `ucdk-capi`)

### 3.1 Principles

1. **The ABI is small and carries bytes.** Types, verbs and fields change by adding protobuf
   fields, not by adding C types. The C surface is a handful of functions over opaque handles and
   `(pointer, length)` byte buffers.
2. **A domain outcome is not a C error.** A ledger that cannot be opened or read is an `Outcome`
   (its `error` arm) and stays data in the response message. The C status code says only whether
   the call itself worked.
3. **No Rust type, no pointer into Rust memory, and no callback crosses the boundary** in version 1.
4. **No panic crosses the boundary** (4.3).

### 3.2 Functions

All functions are `extern "C"`, `#[no_mangle]`, prefixed `ucdk_`. `int32_t` return values are
status codes (4.2). `ucdk_buf` is `{ uint8_t *ptr; size_t len; }`.

```c
/* Version: returns (major << 16) | minor. Callable before anything else. */
uint32_t ucdk_abi_version(void);

/* Session: configuration is a SessionConfig message (ledger kind and location, holder). */
int32_t ucdk_session_open(const uint8_t *config, size_t config_len,
                          ucdk_session **out_session, ucdk_buf *out_error);
void    ucdk_session_close(ucdk_session *session);          /* null is a no-op */

/* The verb. It takes a request message and fills a response message in *out. */
int32_t ucdk_ledger_show(ucdk_session *s, const uint8_t *req, size_t req_len, ucdk_buf *out);

/* Memory: every ucdk_buf the library fills is released here, once. Zeroed buf is a no-op. */
void    ucdk_buf_free(ucdk_buf buf);
```

That is five symbols. A verb is a new symbol and a new minor version; a binding is written once
for the signature every verb shares.

`ucdk_ledger_show` takes a `LedgerShowRequest` (no fields, so any bytes that decode are a valid
request) and answers an `Outcome`: the `ledger_show` arm holds a `LedgerShowResponse` (the
document's token, the highest fence ever granted, the lease if one is recorded and the records in
path order), and the `error` arm holds the reason a ledger could not be opened or read
(`ERROR_CODE_LEDGER`, exit class failed with no effect). A lease is reported `LIVE`, `LAPSING`
(expired but inside the skew bound, so others are still refused) or `LAPSED` by the session's
clock, and `yours` when its holder is `SessionConfig.holder`. The call takes no lease and changes
no file: a file ledger is opened read-only for the call, which creates no lock file, takes no lock
and removes no temporary (9.1). Added in ABI minor 3, when the deployment verbs were removed.

### 3.3 Handles and threads

A session owns one ledger source and the holder name a lease is compared with. `ucdk_session` is
opaque.

- A session runs **one call at a time**. A second call on a busy session returns `UCDK_BUSY` at
  once; it does not queue. Different sessions are independent.
- A call blocks its thread until the library returns. SDKs must not call it on an event-loop
  thread (6.5).
- There is no cancellation in version 1.
- Closing a session with a call in progress is safe (3.6): the call keeps its session alive and
  finishes, and the last holder drops it. Close itself is not fallible. SDKs still close from a
  finalizer only when idle, because the call's answer is the only thing that says what it did.

### 3.4 How protos cross

Requests and responses are serialized protobuf in the proto3 binary format. The caller owns
request bytes for the duration of the call only; the library copies what it keeps. Responses are
bytes allocated by the library.

Everything that crosses is in one package, `ucdk.api.v1` (`SessionConfig`, `LedgerShowRequest`,
`LedgerShowResponse`, `Outcome`, `Error`), owned by `ucdk-api` and compiled by `protox` and
`prost-build` (no `protoc`). The generator (section 6) reads it, so SDK types for requests and
responses are generated too. Lists, never maps, carry the records and their outputs, so every host
prints the same bytes.

`SessionConfig` is data, not environment variables: the ledger kind and location, the holder a
lease is compared with, and, in a test build only, a fixed clock. **Credentials never cross the
ABI as bytes.**

### 3.5 Callbacks

Version 1 has **no callbacks**. A callback would re-enter the host runtime from inside a ledger
write sequence, where a host exception or a GIL or event-loop stall corrupts the reasoning the
write-ahead pattern relies on ("this call returned `Unknown`, so observe"). If a callback is wanted
later it is a second, separate ABI with the same buffer conventions, versioned independently, not a
function-pointer table added to this one. (A provider interface was the reason this section once
existed; it is Historical, section 8.)

### 3.6 As built, and what is left

`crates/ucdk-capi` builds as `cdylib` and `rlib` (the tests call the same functions in-process).
Where the code is more specific than the text above:

- **Five functions.** `abi_version`, `session_open`, `session_close`, `ledger_show` and
  `buf_free`. The header is hand-maintained; `tests/header.rs` compares it with the exports
  (names, order, signatures, constants and the buffer layout), and checks that each name
  resolves, by a linker declaration, to the function of that name.
- **The exported functions are also called from inside the crate** (a unit-test module in
  `src/ffi.rs`). They are `#[unsafe(no_mangle)]`, so every test binary of the crate links its own
  copy of each under one symbol name, and the coverage tool attributes the counts of one copy to
  the source line; which copy it takes follows the build graph, so a function that only the
  integration tests call can read zero lines. The in-crate module makes the crate's own copy
  count whichever one is chosen. The integration tests stay, because they prove the ABI as a host
  sees it.
- **Sessions.** `ledger_kind` is empty or `memory` (a memory ledger, which an empty config also
  gives), or `file` with `ledger_location`. A file ledger is not opened when the session opens:
  each `ucdk_ledger_show` opens it read-only, so opening a session creates no lock file and
  removes no temporary, and a file that cannot be read is the `error` arm of the call that reads
  it. A ledger kind the library lacks is `UCDK_ABI`; a file kind with no location is
  `UCDK_INVALID_ARGUMENT`. `holder` is the name a lease is compared with (empty: a name no lease
  carries). `test_clock_ms`, when not zero, fixes the clock the ledger is judged by, so a test
  chooses when a lease lapses and reads no time. Only a library built with the `test-clock` cargo
  feature honours it (`cargo xtask coverage` and `cargo xtask sdk-test` build with it); the
  default build, which ships, refuses a non-zero value with `UCDK_INVALID_ARGUMENT`, because a
  caller able to fix that clock would defeat the lease.
- **Handles are numbers.** A `ucdk_session *` carries a number from a counter that only grows,
  and the library keeps the map from open numbers to sessions. A closed, twice-closed or
  never-opened handle is refused (`UCDK_INVALID_ARGUMENT`), and stays refused when the allocator
  reuses an address or a later session opens; nothing reads through the pointer. A close during a
  call is safe: the call keeps its session alive and, if it holds the last count, drops it inside
  its `catch_unwind`.
- **Busy and poisoned.** A call takes the session's lock without waiting (`UCDK_BUSY`). A panic
  inside a call leaves the lock poisoned, and every later call on that session is `UCDK_PANIC`
  until it is closed. Containment is `catch_unwind`, so the crate needs `panic = "unwind"` and
  refuses to compile under `panic = "abort"` (a `compile_error!`, held by `tests/panic_unwind.rs`).
- **Freeing a buffer twice is undefined**, as 4.1 says; it is not detected. A zeroed buffer, or one
  with a zero length, is ignored.
- **Not done.** The library does not check an SDK's version: the loader rule of section 5 is the
  SDK's (both loaders do it: `sdk/python/ucdk/_library.py`, `sdk/typescript/src/library.ts`). The
  `UCDK_ABI` refusal of a different major (10.3) therefore has no library side yet. A version
  script is not used (a Rust `cdylib` exports only its `#[no_mangle]` functions, but the built
  library's symbol table is not read). The built `cdylib` loaded dynamically, Miri and the
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
with no effect, 6 failed after changing something, 8 a sweep found objects no record accounts
for): `ucdk-api` holds the one table from an exit class to its code (`ExitClass::code`) and
`ledger_show` the one producer of the answer and its class, the CLI prints the code as its exit
status, and each SDK returns the class as a field of the result. There is one table, not one per
host. Only done and failed with no effect have a producer today; a function from the verdicts of a
later verb to a class is written with that verb.

### 4.3 No panics across FFI

Every exported function body runs inside `std::panic::catch_unwind`. The workspace forbids
`unsafe` today (`unsafe_code = "forbid"` in the root `Cargo.toml`); the C ABI crate is the single
exception, by a per-crate lint override that `cargo xtask check` pins (it fails if any other
crate sets it, and if the override lives anywhere but that crate's manifest). The `cdylib` is
built with `panic = "unwind"` so containment works; a contained panic poisons the session (the
call may be part way through, and a write-ahead ledger, not in-memory state, is the recovery
path) and returns `UCDK_PANIC`. Panics in a destructor, and a panic while the panic message is
being built, abort the process by Rust's own rule; the tests in 10.3 pin that this path is not
reachable from the ABI.

## 5. ABI versioning and compatibility (partly exists)

- **Two numbers.** `ucdk_abi_version()` returns `(major << 16) | minor`. The generated header
  defines `UCDK_ABI_MAJOR` and `UCDK_ABI_MINOR`. Adding a function or a verb bumps the minor;
  removing or changing the signature of an existing function, the status-code meanings, or the
  ownership rules bumps the major. The major starts at 1 on the first release; before that the
  library is `0.x` and nothing is promised.
- **Loader rule.** An SDK built for `(M, m)` accepts a library with the same `M` and, **while `M`
  is 0**, the same `m`: nothing is promised before the first release, and a minor may remove a
  function as well as add one (ABI 0.3 removed six), so under the rule "a minor at least the
  SDK's" a stale SDK would pass the check and fail at the lookup of a symbol that is gone. From
  major 1 a minor only adds, and the library's minor must be at least the SDK's. The loader
  checks this first thing after loading, before any other call, and fails with an error naming
  both versions and the library. An SDK and a library that differ are refused, never guessed at.
  (`UCDK_ABI_MINOR` is 3 and `SDK_ABI` is `(0, 3)` in both SDKs; the header's comment records the
  rule.)
- **Payload compatibility is the protobuf rules.** Fields are added with new numbers; a removed
  field's number and name are `reserved` (`api.proto` does this for the deployment messages it
  retired); an unknown field is ignored on read. A change that breaks that is a new package
  version (`ucdk.api.v2`) that coexists with `v1` in the same library; `v1` messages keep working
  until the major version that drops them.
- **A breaking-change check on the protos** is added to `cargo xtask check`: it compares the
  descriptor set against the one recorded for the last release and fails on a removed or
  renumbered field, a changed type, or a removed enum value. It is a Rust program over
  descriptors (no extra tool).
- **The bytes a ledger file holds are part of the compatibility surface.** The record lines of a
  file ledger are pinned by `sdk/ledger-golden/records.ucdk` (every state and origin, with a
  physical id, dependencies and outputs), the lease lines by `leased.ucdk`, and the key of a
  record path by a golden text in `ucdk-core`. A change to them is a deliberate, separately
  reviewed change, because a ledger written earlier must still read.
- **Symbols.** The library exports only `ucdk_*` symbols (a version script on Linux, an exports
  list on macOS), and a test reads the built library's exported symbol table and compares it with
  the header.

## 6. How SDKs are generated (exists for Python and TypeScript: `ucdk-codegen`)

### 6.1 Input

The descriptor set for `ucdk.api.v1`, produced in-process by `protox` with source info (comments)
included. This is the same compile `ucdk-api/build.rs` does, so no `protoc` and no per-language
protobuf toolchain is needed to generate anything.

### 6.2 What is generated, and what is hand-written

| Generated, never edited | Hand-written, small and fixed |
|---|---|
| message classes and enums, with docs taken from the proto comments | the C ABI binding: load the library, check the version, call the five functions |
| binary encode and decode for those messages | the `Session` type: `open`, `ledger_show`, `close`, and the error type built from the status codes |
| | locating the library (6.5) |
| | the worker thread and the promises (TypeScript) |
| | packaging metadata |

The hand-written part does not grow when a message is added. A test enforces that (10.4).

### 6.3 Encoding: generated, no protobuf runtime dependency (decided, built)

The generator emits the encoder and decoder itself (option B of the design): an SDK has no
protobuf runtime dependency, no per-language protobuf generator and no version skew. The subset
of proto3 it writes code for is the one the protos use: strings, booleans, `int32`, `int64`,
`uint32`, `uint64`, enums, messages, `repeated` strings and messages, `map<string, T>`, `oneof`
and `optional`; any other field type is refused by name when the generator reads it (a new proto
feature is a decision in `ucdk-codegen/src/model.rs`, not a field an SDK silently drops). What
the targets read from the descriptors (the field types and the order a message is written in) is
shared between them in `model.rs`; what they write is not.
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
unless the goldens set every field of every message of the package, and its count of those fields
is an equality, so a field added without a golden fails by name and by count.

### 6.4 What "adding a resource" cost (Historical)

The generator once also wrote typed templates for the resource kinds, read from message and field
options of a `ucdk.resource.v1` catalog, and lowering of a resource list to nodes was driven by
the same descriptors, so a new kind was a proto message and a provider. The catalog, the lowering
and the templates were removed with the deployment side; the generator now writes message
classes and enums only, and a new message costs no hand-written SDK source. The history is in git:
`git log --diff-filter=D -- crates/ucdk-plan crates/ucdk-proto`.

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
committed still holds for the Rust types (`ucdk-api/build.rs` writes them into the build
directory); the reason it does not for an SDK is that a Python package and TypeScript source are
used from a checkout without a build step.

The generator's own output for a fixed proto (`crates/ucdk-codegen/tests/proto`, every supported
field shape, comments, names that need care) is kept as golden files in
`crates/ucdk-codegen/tests/golden`, held by a Rust test and loaded by the Python and the TypeScript tests (the files of both
targets are in one directory, since their paths differ).

## 7. Resources as protos (Historical)

`ucdk.resource.v1` defined `Resource { id, uses, oneof body { Service, Job } }`, the references
between resources and the options that drove lowering, and a plan was hashed over the lowered
nodes. The package, the planner (`ucdk-plan`) and the refusal of a kind a provider does not host
were removed with the deployment side; the git history before that removal has them
(`git log --diff-filter=D -- crates/ucdk-proto crates/ucdk-plan`). Two pieces were kept and moved
into `ucdk-core`: the ordering by dependency (`order`) and the length-framed hash (`Framed`).

## 8. Providers (Historical)

A provider was a Rust crate implementing a `Provider` trait (`observe`, `create`, `update`,
`delete`, `list_owned`, `list_stamped`), checked by a contract kit, with an in-memory fake with
fault injection as the executable spec. The trait, the kit and the fake were removed with the
deployment side (`git log --diff-filter=D -- crates/ucdk-provider crates/ucdk-provider-fake
crates/ucdk-engine`). What was kept is the pattern, not the code: write the intent before a
change that lands somewhere the ledger does not own, confirm it after, resolve a leftover intent
by looking, and test a crash at every call by running again after stopping at call N. The lease
lifecycle of the engine moved into `ucdk-ledger` as `LeaseGuard` (9.2).

## 9. State and the ledger

**Exists:** `Ledger` is a single document with a version `Token`; a write names the token it read
and fails with `LedgerError::Stale` if another writer got there first. `MemoryLedger` keeps the
document in the process, for tests. `FileLedger` keeps it in a file, so a run can be resumed by
another process on the same machine (below). Both keep a lease in the document (9.2): a holder
takes it, and every ledger write it makes carries the lease's fence. A read returns a `Snapshot`:
the records, the token, the highest fence ever granted, the lease as recorded (a `Holding`, which
says whether it is live, lapsing or lapsed at a given instant) and the ledger's own clock reading
taken with the read.

A record is a path (`RecordPath`; a record of a release is named by a key hashed from a readable
name, `RecordKey`, in a domain of its own so a name and a path spelled alike never share a key),
a `Tag` for its kind, a readable name, an origin (created or adopted), a state (intent, confirmed
or reaping), a `Locator` for where the thing is, the paths it depended on and a plain string map
of values. A ledger holds no secret: a writer puts none in that map.

**Decided direction:** state that a team shares is stored in the environment it concerns, in
storage that environment owns, so that whoever can reach it with the right credentials sees the
same state, and no machine-local file is the source of truth.

**Proposals, in order:**

1. **Done: the file ledger** (`FileLedger`; single machine, not a team backend), so a run can
   resume after a crash. See 9.1.
2. **Done: the lease** with an injected clock and fencing in the ledger write, so a stalled
   holder cannot write after a newer one took over. Fencing protects the ledger, not what a
   holder does outside it. See 9.2.
3. A networked ledger on conditional writes (create-only, then if-match), with the lease in the
   same document. A store that cannot give conditional writes is refused as a ledger home.

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
- **Opened read-only.** `FileLedger::open_read_only(path)` (and `open_read_only_with_clock`, which
  reports `Snapshot::at` by the clock it is given) reads the document and nothing else: it
  creates no `<path>.lock`, takes no lock and removes no stale `<path>.tmp`, so it works in a
  directory it may not write and contends with no writer (a write replaces the file whole, so a
  read sees the old document or the new one). Every write, renew and release answers
  `LedgerError::ReadOnly` and every acquire `LeaseError::ReadOnly`. The file must exist: a
  missing file is an error, not an empty ledger. This is how `ledger show` reads a file.
- **Escaping.** `escape_token` is the function the file writer uses for every value, and is
  public so that anything that prints a value of a ledger (the `ucdk ledger show` text) escapes
  it the same way and cannot drift from the file.
`SessionConfig` names the ledger by kind and location.

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
  version alone, so a token read before the lease was taken stays valid for a write under it.
- **Unfenced writes.** `Ledger::write` is for a document nobody leases: it is refused as
  `LedgerError::Leased` while a lease is live (or lapsed less than the skew bound ago).
- **The lease guard.** `LeaseGuard` is the lifecycle of one holder's lease, and takes no clock:
  the ledger owns the clock and judges expiry, and the guard reads time only through
  `Snapshot.at`. `LeaseGuard::acquire(ledger, holder, ttl)` is done after the refusals that need no
  ledger; `commit(change)` is a fenced write that reads the document again and tries again on a
  stale token, up to five times, then answers `GuardError::Busy`; `renew()` extends the lease
  (a holder renews before each piece of work that must not outlast it); `release()` gives it back,
  however the work ended. `GuardError` says what the caller may assume: `Held` (another holder,
  with the longest wait; nothing changed), `Lost` (the fence or the lease is gone: every later
  call answers `Lost` and writes nothing, so a holder that lost its lease cannot write over the
  new holder's work), `Unconfirmed` (a grant or a release whose answer was lost: the lease may
  still be held, and until it lapses a different holder is refused and this one takes it again at
  once; treat it as an effect that may have landed), `Ledger` (any other failure), `Busy` and
  `NotHeld` (after a release). `DEFAULT_LEASE_TTL` is ten minutes and `default_holder()` is the
  process id and a random number, so two holders on one ledger are told apart even when their
  process ids are equal. A holder that stops at `Lost` leaves what it had begun to the next
  holder, who resolves it from the intent record.
- **Naming the holder.** `SessionConfig.holder` (both SDKs) and `ucdk ledger show --holder` name
  the holder a lease is compared with (`yours`); unnamed, the default holder is used, which no
  lease carries. Name the holder a lease was taken under to see it as your own.
- **Not yet.** The lease length is not in `SessionConfig`, and no verb takes, renews or breaks a
  lease by hand. `ledger show` takes none.

## 10. Testing plan

Rules that already bind every layer: tests are Rust, a bug fix ships with a test that failed
first, and `cargo xtask coverage` requires every crate on its own to reach **99% of lines and 99%
of branches** (generated code and a `main.rs` excluded). A rule below that goes beyond what the
repository does today is a **proposal**.

### 10.1 Per layer

| Layer | What is tested | How | Defect class it catches |
|---|---|---|---|
| `ucdk-core` | record paths and keys (the key of a path is pinned to its scheme, a name key to its own), hashes read back from text and fed with framing, dependency order, the clock | unit and golden tests (`tests/model.rs`, `framing.rs`, `hash_text.rs`, `order.rs`) | a changed key, two fields that run together in a hash, a missed cycle |
| `ucdk-ledger` | CAS semantics, record encoding, the lease and fence, the read-only open, the lease guard | unit tests; a conformance suite (`tests/conformance.rs`) and the lease suite (`tests/lease.rs`) run on both implementations; file-format and crash tests (`tests/file.rs`); the lease guard on both with a counter for the calls it guards (`tests/guard.rs`); the golden files (`tests/golden.rs`) | lost update, stale write accepted, a torn or altered file read as a ledger, a holder that writes after it lost the lease, a read that changes a file |
| `ucdk-api` | the ledger show over a snapshot: every state and origin, the orders, the lease by the clock it is given, every way a ledger cannot be read; the table of exit classes | `tests/ledger_show.rs`, `tests/exit.rs`, the wire goldens (`tests/sdk_golden.rs`) | a field dropped in conversion, a lease judged by the wrong clock |
| `ucdk-cli` | arguments, exit codes, the text of a ledger show | in-process runs over a `World` with an injected clock (`tests/cli.rs`, `tests/render.rs`) | wrong exit class, a value that forges a line of the text |
| `ucdk-capi` | the five functions, called as C would | see 10.3 | ownership, panic, version and malformed-input defects |
| SDKs | binding and generated code | see 10.4 | an SDK that disagrees with the core or with another SDK |
| generator | output for fixture protos, including a package that imports another; the committed SDK | golden output (`tests/generate.rs`, `golden_coverage.rs`), the `sdk-fresh` gate, and the fixture's golden loaded and run by the Python and TypeScript tests | generated code that does not load or encode wrongly, a refused-in-silence field type, a stale committed SDK |

### 10.2 Crash and fault injection

The write-ahead pattern's central claim is that a crash at any point leaves a state the next run
resolves by looking. It was tested (Historical, 7 and 8) by running a second run over the same
world after stopping the first at call N, for every N, against each fault a fake could inject.
The ledger keeps what that needs: the lease tests run on both ledgers with a clock the test moves
(a holder that dies and never releases is refused while the lease holds, then the clock is moved
past the lease and the skew bound and the next holder is let in; a holder whose clock is moved
past its lease mid-run is refused by the ledger at its next write and changes nothing more), and
`tests/guard.rs` holds the same for `LeaseGuard`. The crash sweep itself is written again over the
next engine, with the pattern kept.

### 10.3 ABI conformance (in-process part exists)

Run from Rust, in-process, against the exported functions (so they count toward coverage) and
again against the built `cdylib` loaded dynamically:

- every function with null, zero-length, oversized and truncated inputs; a handle used after
  close; a double close; `ucdk_buf_free` on a zeroed buffer;
- a ledger that panics (a test-only wrapper around a memory ledger, `HookedLedger`) must give
  `UCDK_PANIC` and a poisoned session, never an abort or an unwind into the caller; the same
  wrapper blocks a call to show that a second call is `UCDK_BUSY`, and closes the session during a
  call to show that a destructor that panics is contained;
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
- **Verb contract:** `ledger_show` is run through each SDK over the ledger goldens of
  `sdk/ledger-golden` (copied first, so a session can never change the golden): the document of
  `records.ucdk`, the lease of `leased.ucdk` live, lapsing and lapsed by the clock the session is
  opened with, the lease as `yours` for a session named as its holder, and a file that is not a
  ledger, or is missing, as the ledger error. The command line's text of the same goldens is held
  by `crates/ucdk-cli/tests/cli.rs`.
- **End to end:** each SDK runs the verb through the real shared library (`cargo xtask sdk-test`
  builds `ucdk-capi` first). Both SDKs' tests also hold the loader (a wrong major, another minor,
  a stale SDK refused by name, a file that is not the library), a closed session (refused by the
  SDK, and a closed handle refused by the library) and every status code's error; Python's also
  holds the search of the package directory, which the TypeScript SDK does not have yet (it takes
  `libraryPath` only, until the package layout of step 15).
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

### 10.5 Real clouds (Historical)

There are no real providers for now. The rule stands for any that is added: a live run is behind
an explicit flag that refuses to run without a named account and a spend cap, recordings are
scrubbed before they are committed, and live tests never run in pull-request CI.

### 10.6 What runs where

| Suite | Pull request | After merge / on demand |
|---|---|---|
| fmt, clippy, deny, SPDX, run-lines, dependency direction, public lint, sign-off (`cargo xtask check`) | yes | yes |
| unit, golden, ledger, CLI; 99% line and branch per crate (`cargo xtask coverage`) | yes | yes |
| ABI conformance in-process; symbol table check | yes | yes |
| SDK conformance on Linux x86-64 for each SDK (`cargo xtask sdk-test`) | yes | all supported platforms |
| built `cdylib` loaded dynamically, Miri, sanitizer builds | no | yes |
| macOS arm64 and Linux arm64 builds and SDK conformance | no | yes |

The workflow has three jobs (`check`, `coverage`, `sdk`), one `cargo xtask` verb per job as the
rule requires.

## 11. Open questions

These are the decisions this document cannot make alone:

1. **SDK-language tests** (10.4): decided: a scoped exception for `sdk/<language>`, run through
   `cargo xtask sdk-test` (`CONTRIBUTING.md`).
2. **Encoding strategy** (6.3): decided: generated encoders.
3. **A networked ledger** (9): which store, and how the lease is kept in the same document.

## 12. Implementation sequence (Historical)

This was the order the framework was built in, each step one pull request. All of it exists
except the packaging step. Steps that built the deployment side (a planner with descriptor-driven
lowering, a read-only status, a leak sweep, the lease inside an engine) were removed with it, and
what each left behind is named in the section it belongs to.

1. The `ucdk.api.v1` messages and one table of exit classes (`ucdk-api`).
2. The file ledger and the ledger conformance suite (`ucdk-ledger`, 9.1).
3. The C ABI: the `cdylib`, the header, panic containment and the in-process conformance tests
   (`ucdk-capi`, 3 and 10.3), with the per-crate `unsafe` exemption and its gate.
4. The generator and `cargo xtask gen-sdk` (`ucdk-codegen`), the wire goldens, and the scoped
   exception for SDK-language tests with its `sdk` job.
5. The Python SDK (`sdk/python`) and the TypeScript SDK (`sdk/typescript`), each with the loader
   rule of section 5, `Session`, one error type per status code and the tests of 10.4.
6. The lease and the fence with an injected clock (9.2), and the lease lifecycle as `LeaseGuard`.
7. The deletion of the deployment side: the shared values moved to `ucdk-core`, the ledger
   generalised (the read-only open, the snapshot's fence, lease and clock, the escape function),
   `ledger show` added in the library, the C ABI (minor 3), the command line and both SDKs, and
   the ABI loader rule changed to an exact minor while the major is 0.
8. **Release packaging** (a proposal): per-platform library builds, wheels and npm packages, the
   version policy of section 5 enforced by `cargo xtask check`.
