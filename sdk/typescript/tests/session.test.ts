// SPDX-License-Identifier: Apache-2.0
/**
 * The session over the real library: closing, the errors the library answers with, and the
 * buffers that cross the boundary.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import * as sdk from "../src/index.ts";
import { Session, UcdkError, api, errors, resources } from "../src/index.ts";
import {
  Busy,
  DecodeFailed,
  Internal,
  InvalidArgument,
  LibraryError,
  Panic,
  SessionClosed,
  TooLarge,
  Unsupported,
  errorFor,
} from "../src/errors.ts";
import { Library } from "../src/library.ts";
import { IMAGE, library, libraryPath, openSession, pair } from "./support.ts";

// ---- closing -------------------------------------------------------------------------------

test("a closed session refuses every verb", async () => {
  const wanted = pair();
  const session = await openSession();
  assert.equal(session.closed, false);
  await session.close();
  assert.equal(session.closed, true);
  for (const call of [
    () => session.loadCatalog(wanted),
    () => session.plan(wanted),
    () => session.apply(wanted, "a".repeat(64)),
    () => session.destroy(wanted, "a".repeat(64)),
    () => session.status(wanted),
    () => session.leaks(),
  ]) {
    await assert.rejects(call(), SessionClosed);
  }
});

test("closing twice is not an error", async () => {
  const session = await openSession();
  await session.close();
  await session.close();
});

test("a session is disposed of by await using", async () => {
  let kept: Session;
  {
    await using session = await openSession();
    kept = session;
    assert.equal(session.closed, false);
  }
  assert.equal(kept.closed, true);
});

test("calls made before a close finish, and the ones after it are refused", async () => {
  const wanted = pair();
  const session = await openSession();
  const planned = session.plan(wanted);
  const closing = session.close();
  await assert.rejects(session.plan(wanted), SessionClosed);
  assert.ok((await planned).planned);
  await closing;
});

test("the library refuses a handle that was closed", () => {
  const lib = library();
  const handle = lib.openSession(new api.SessionConfig({ provider: "fake" }).encode());
  lib.closeSession(handle);
  assert.throws(() => lib.call("plan", handle, new api.PlanRequest().encode()), (error: unknown) => {
    assert.ok(error instanceof InvalidArgument);
    assert.equal(error.status, 1);
    return true;
  });
  // Closing a handle again, or one never opened, is ignored.
  lib.closeSession(handle);
  lib.closeSession(12345n);
});

test("a sessions calls do not reach another session", async () => {
  const wanted = pair();
  await using first = await openSession();
  await using second = await openSession();
  const plan = (await first.plan(wanted)).planned;
  assert.ok(plan);
  await first.apply(wanted, plan.hash);
  const records = async (s: Session) =>
    (await s.status(wanted)).status?.objects.map((o) => o.record);
  assert.deepEqual(await records(first), [api.RecordState.CONFIRMED, api.RecordState.CONFIRMED, api.RecordState.CONFIRMED]);
  assert.deepEqual(await records(second), [undefined, undefined, undefined]);
});

test("the calls of one session run one at a time, in the order they were made", async () => {
  // Two applies of one plan, made without waiting: the first creates, the second finds it done.
  const wanted = pair();
  await using session = await openSession();
  const hash = (await session.plan(wanted)).planned?.hash as string;
  const [one, two] = await Promise.all([session.apply(wanted, hash), session.apply(wanted, hash)]);
  assert.equal(one.whichResult(), "done");
  assert.deepEqual(one.done?.created.length, 3);
  // The second plan is no longer the one that was pinned, so it is refused as data, not as BUSY.
  assert.equal(two.whichResult(), "refused");
});

test("a session whose open failed can be collected", async () => {
  await assert.rejects(
    Session.open(new api.SessionConfig({ provider: "nope" }), { libraryPath: libraryPath() }),
    Unsupported,
  );
});

test("a library that cannot be loaded or has the wrong version fails the open", async () => {
  await assert.rejects(
    Session.open(new api.SessionConfig({ provider: "fake" }), { libraryPath: "/no/such/lib.so" }),
    (error: unknown) => {
      assert.ok(error instanceof LibraryError);
      assert.match(error.message, /\/no\/such\/lib\.so/);
      return true;
    },
  );
  const libc = process.platform === "darwin" ? "/usr/lib/libSystem.B.dylib" : "libc.so.6";
  await assert.rejects(
    Session.open(new api.SessionConfig({ provider: "fake" }), { libraryPath: libc }),
    /ucdk_abi_version/,
  );
});

// ---- statuses ------------------------------------------------------------------------------

test("a provider this library does not have is unsupported", async () => {
  await assert.rejects(openSession("no-such-provider"), (error: unknown) => {
    assert.ok(error instanceof Unsupported);
    assert.equal(error.status, 4);
    return true;
  });
});

test("provider options this library does not know are unsupported", async () => {
  await assert.rejects(
    openSession("fake", { providerOptions: { region: "r1" } }),
    Unsupported,
  );
});

test("bytes that are not the message are a decode error", () => {
  const lib = library();
  const handle = lib.openSession(new api.SessionConfig({ provider: "fake" }).encode());
  try {
    assert.throws(() => lib.call("plan", handle, Uint8Array.of(0x0a, 0xff, 0xff)), (error: unknown) => {
      assert.ok(error instanceof DecodeFailed);
      assert.equal(error.status, 2);
      assert.ok(error.error?.message);
      return true;
    });
  } finally {
    lib.closeSession(handle);
  }
});

test("a request over the size limit is too large", () => {
  const lib = library();
  const handle = lib.openSession(new api.SessionConfig({ provider: "fake" }).encode());
  try {
    assert.throws(() => lib.call("plan", handle, new Uint8Array(64 * 1024 * 1024 + 1)), (error: unknown) => {
      assert.ok(error instanceof TooLarge);
      assert.equal(error.status, 5);
      return true;
    });
  } finally {
    lib.closeSession(handle);
  }
});

test("an empty request is the empty message", () => {
  const lib = library();
  const handle = lib.openSession(new api.SessionConfig({ provider: "fake" }).encode());
  try {
    const catalog = api.CatalogResponse.decode(lib.call("load_catalog", handle, new Uint8Array(0)));
    assert.deepEqual([catalog.nodes, catalog.findings], [[], []]);
  } finally {
    lib.closeSession(handle);
  }
});

test("a response is copied out of the library's buffer, which is freed each time", () => {
  // A loop of calls whose buffers are all released: if the free were wrong the process would
  // crash or the bytes of an earlier response would show in a later one.
  const lib = library();
  const handle = lib.openSession(new api.SessionConfig({ provider: "fake" }).encode());
  try {
    const request = new api.PlanRequest({ resources: pair() }).encode();
    const first = lib.call("plan", handle, request);
    for (let i = 0; i < 200; i += 1) {
      assert.deepEqual(lib.call("plan", handle, request), first);
    }
  } finally {
    lib.closeSession(handle);
  }
});

test("a verb the library answers with a status is the error of that status", async () => {
  await using session = await openSession();
  // Over the limit of a request, which the library refuses before it reads a byte of it.
  const huge = resources.resourceList(
    new resources.Service("big", IMAGE, { args: ["x".repeat(64 * 1024 * 1024 + 1)] }),
  );
  await assert.rejects(session.plan(huge), (error: unknown) => {
    assert.ok(error instanceof TooLarge);
    assert.equal(error.status, 5);
    assert.match(error.message, /^ucdk_plan failed with status 5 \(TooLarge\)/);
    return true;
  });
  // The session is as it was.
  assert.equal((await session.plan(pair())).whichResult(), "planned");
});

// ---- the table from a status code to an error ----------------------------------------------

test("each status has its class", () => {
  for (const [status, cls] of [
    [1, InvalidArgument],
    [2, DecodeFailed],
    [3, Busy],
    [4, Unsupported],
    [5, TooLarge],
    [100, Panic],
    [101, Internal],
  ] as const) {
    const error = errorFor("ucdk_plan", status, new Uint8Array(0));
    assert.equal(error.constructor, cls, String(status));
    assert.equal(error.status, status);
    assert.ok(error instanceof UcdkError);
    assert.ok(error instanceof Error);
    assert.equal(error.name, cls.name);
    assert.match(error.message, new RegExp(`^ucdk_plan failed with status ${status} `));
  }
});

test("a status the sdk does not know is still an error carrying the number", () => {
  const error = errorFor("ucdk_apply", 77, new Uint8Array(0));
  assert.equal(error.constructor, UcdkError);
  assert.equal(error.status, 77);
});

test("the message of the library is kept", () => {
  const detail = new api.Error({ code: api.ErrorCode.PROVIDER, message: "the ledger is held" });
  const error = errorFor("ucdk_session_open", 101, detail.encode());
  assert.equal(error.error?.message, "the ledger is held");
  assert.ok(error.message.endsWith(": the ledger is held"));
});

test("detail that is not an error message is dropped, not thrown", () => {
  const error = errorFor("ucdk_plan", 101, Uint8Array.of(0x0a, 0xff));
  assert.equal(error.error, undefined);
  assert.equal(error.constructor, Internal);
});

test("a detail with no message adds nothing", () => {
  const detail = new api.Error({ code: api.ErrorCode.PROVIDER });
  const error = errorFor("ucdk_plan", 3, detail.encode());
  assert.ok(error.message.endsWith("(Busy)"));
});

test("the errors the sdk makes itself carry no status", () => {
  for (const error of [new SessionClosed("x"), new LibraryError("x"), new UcdkError("x")]) {
    assert.equal(error.status, 0);
    assert.equal(error.error, undefined);
  }
});

// ---- what the package exports --------------------------------------------------------------

test("the package names what a user needs", () => {
  assert.equal(sdk.Library, Library);
  assert.equal(sdk.Session, Session);
  assert.equal(sdk.UcdkError, UcdkError);
  assert.equal(sdk.errors.SessionClosed, errors.SessionClosed);
  for (const name of ["api", "proto", "resources", "errors", "SDK_ABI"] as const) {
    assert.ok(name in sdk, name);
  }
});
