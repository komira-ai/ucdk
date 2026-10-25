# ucdk for TypeScript

The TypeScript SDK of ucdk: typed resources, and a session over the ucdk shared library. It has no
dependency: not on a protobuf package, not on a native addon. It binds the library with Node's
built-in `node:ffi`, so it needs **Node 26.12 or newer** (`node:ffi` is experimental there and
prints a warning once). It is TypeScript source that Node runs by stripping the types.

```ts
import { Session, api, proto, resources } from "ucdk";

const db = new resources.Service("db", proto.Image.digest("sha256:..."), { port: 5432 });
const web = new resources.Service("web", proto.Image.output("build", "img"), {
  env: { DB: db.url }, // a typed reference to an output of db
  uses: [db],
  exposure: new resources.Service.Public(),
});
const wanted = resources.resourceList(web, db);

await using session = await Session.open(
  new api.SessionConfig({ provider: "fake", ledgerKind: "memory" }),
  { libraryPath: "target/debug/libucdk_capi.so" },
);
const plan = await session.plan(wanted); // an Outcome; changes nothing
const done = await session.apply(wanted, plan.planned!.hash); // refused unless the plan is still this one
console.log(done.done?.created, done.exit);
```

- The library is named by `libraryPath`; there is no environment variable and no default.
- The loader checks the library's ABI version before it calls anything else: the same major, and a
  minor at least the SDK's (`SDK_ABI`, the header's `UCDK_ABI_MAJOR`/`MINOR`).
- Every call runs on a worker thread that belongs to the session, so a call that blocks until the
  engine returns blocks the worker, not your event loop, and a session returns promises. The
  calls of one session run one at a time, in the order they were made. Close the session (or
  `await using` it): it holds a thread and, with a file ledger, the ledger.
- A plan the engine refuses, a conflict and a run that fails half way are data in the `Outcome`
  (`outcome.refused`, `outcome.failed`, `outcome.exit`), never errors. An error (`errors`, one
  class per status code of the library) means the call itself did not work.
- 64 bit integers are `bigint`s; the rest are `number`s. An enum is a constant object
  (`api.ExitClass.DONE`) and its type. A field is named in camel case (`planHash`).
- `src/generated` is written by `cargo xtask gen-sdk` from the protos and is not edited by hand.
  Everything else in `src` is hand-written and small: `library.ts` (the nine C functions),
  `session.ts` and `worker.ts`, and `errors.ts`.

Tests are in `tests`, use only Node's built-in test runner, and run with `cargo xtask sdk-test`,
which builds the library and runs them on the pinned Node.
