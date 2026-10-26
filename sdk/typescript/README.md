# ucdk for TypeScript

The TypeScript SDK of ucdk: a session over the ucdk shared library. It has no dependency: not on a
protobuf package, not on a native addon. It binds the library with Node's built-in `node:ffi`, so
it needs **Node 26.12 or newer** (`node:ffi` is experimental there and prints a warning once). It
is TypeScript source that Node runs by stripping the types.

```ts
import { Session, api } from "ucdk";

await using session = await Session.open(
  new api.SessionConfig({ ledgerKind: "file", ledgerLocation: "ledger.ucdk" }),
  { libraryPath: "target/debug/libucdk_capi.so" },
);
const shown = await session.ledgerShow(); // an Outcome; takes no lease and changes no file
console.log(shown.ledgerShow?.token, shown.ledgerShow?.fence, shown.ledgerShow?.lease);
```

- The library is named by `libraryPath`; there is no environment variable and no default.
- The loader checks the library's ABI version before it calls anything else: the same major and,
  while the major is 0, the same minor (a minor of major 0 may remove a function); from major 1 a
  minor at least the SDK's (`SDK_ABI`, the header's `UCDK_ABI_MAJOR`/`MINOR`).
- Every call runs on a worker thread that belongs to the session, so a call that blocks until the
  library returns blocks the worker, not your event loop, and a session returns promises. The
  calls of one session run one at a time, in the order they were made. Close the session (or
  `await using` it): it holds a thread.
- A ledger that cannot be read is data in the `Outcome` (`outcome.error`, `outcome.exit`), never
  an error. An error (`errors`, one class per status code of the library) means the call itself
  did not work.
- 64 bit integers are `bigint`s; the rest are `number`s. An enum is a constant object
  (`api.ExitClass.DONE`) and its type. A field is named in camel case (`expiresMs`).
- `src/generated` is written by `cargo xtask gen-sdk` from the protos and is not edited by hand.
  Everything else in `src` is hand-written and small: `library.ts` (the five C functions),
  `session.ts` and `worker.ts`, and `errors.ts`.

Tests are in `tests`, use only Node's built-in test runner, and run with `cargo xtask sdk-test`,
which builds the library and runs them on the pinned Node.
