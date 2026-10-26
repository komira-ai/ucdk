// SPDX-License-Identifier: Apache-2.0
/**
 * The session over the real library: closing, the errors the library answers with, and the
 * buffers that cross the boundary.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import * as sdk from "../src/index.ts";
import { Session, UcdkError, api, errors } from "../src/index.ts";
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
import { WHILE_HELD_MS, library, libraryPath, openSession, withGolden } from "./support.ts";

// ---- closing -------------------------------------------------------------------------------

test("a closed session refuses its verb", async () => {
  const session = await openSession();
  assert.equal(session.closed, false);
  await session.close();
  assert.equal(session.closed, true);
  await assert.rejects(session.ledgerShow(), SessionClosed);
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
  const session = await openSession();
  const shown = session.ledgerShow();
  const closing = session.close();
  await assert.rejects(session.ledgerShow(), SessionClosed);
  assert.ok((await shown).ledgerShow);
  await closing;
});

test("the library refuses a handle that was closed", () => {
  const lib = library();
  const handle = lib.openSession(new api.SessionConfig().encode());
  lib.closeSession(handle);
  assert.throws(() => lib.call("ledger_show", handle, new api.LedgerShowRequest().encode()), (error: unknown) => {
    assert.ok(error instanceof InvalidArgument);
    assert.equal(error.status, 1);
    return true;
  });
  // Closing a handle again, or one never opened, is ignored.
  lib.closeSession(handle);
  lib.closeSession(12345n);
});

test("a sessions calls do not reach another session", () =>
  withGolden("records.ucdk", async (golden) => {
    await using first = await openSession({ ledgerKind: "file", ledgerLocation: golden.path });
    await using second = await openSession();
    assert.equal((await first.ledgerShow()).ledgerShow?.records.length, 5);
    assert.deepEqual((await second.ledgerShow()).ledgerShow?.records, []);
  }));

test("the calls of one session run one at a time, in the order they were made", async () => {
  await using session = await openSession();
  const answers = await Promise.all([session.ledgerShow(), session.ledgerShow(), session.ledgerShow()]);
  for (const answer of answers) {
    assert.equal(answer.whichResult(), "ledgerShow");
  }
});

test("a session whose open failed can be collected", async () => {
  await assert.rejects(
    Session.open(new api.SessionConfig({ ledgerKind: "tape" }), { libraryPath: libraryPath() }),
    Unsupported,
  );
});

test("a library that cannot be loaded or has the wrong version fails the open", async () => {
  await assert.rejects(
    Session.open(new api.SessionConfig(), { libraryPath: "/no/such/lib.so" }),
    (error: unknown) => {
      assert.ok(error instanceof LibraryError);
      assert.match(error.message, /\/no\/such\/lib\.so/);
      return true;
    },
  );
  const libc = process.platform === "darwin" ? "/usr/lib/libSystem.B.dylib" : "libc.so.6";
  await assert.rejects(Session.open(new api.SessionConfig(), { libraryPath: libc }), /ucdk_abi_version/);
});

// ---- statuses ------------------------------------------------------------------------------

test("a ledger kind this library does not have is unsupported", async () => {
  await assert.rejects(openSession({ ledgerKind: "tape" }), (error: unknown) => {
    assert.ok(error instanceof Unsupported);
    assert.equal(error.status, 4);
    return true;
  });
});

test("a file ledger with no location is an invalid argument", async () => {
  await assert.rejects(openSession({ ledgerKind: "file" }), (error: unknown) => {
    assert.ok(error instanceof InvalidArgument);
    assert.equal(error.status, 1);
    return true;
  });
});

test("an empty config opens a memory ledger", async () => {
  await using session = await Session.open(new api.SessionConfig(), { libraryPath: libraryPath() });
  assert.equal((await session.ledgerShow()).ledgerShow?.token, "v0");
});

test("bytes that are not the message are a decode error", () => {
  const lib = library();
  const handle = lib.openSession(new api.SessionConfig().encode());
  try {
    assert.throws(() => lib.call("ledger_show", handle, Uint8Array.of(0x0a, 0xff, 0xff)), (error: unknown) => {
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
  const handle = lib.openSession(new api.SessionConfig().encode());
  try {
    assert.throws(() => lib.call("ledger_show", handle, new Uint8Array(64 * 1024 * 1024 + 1)), (error: unknown) => {
      assert.ok(error instanceof TooLarge);
      assert.equal(error.status, 5);
      assert.match(error.message, /^ucdk_ledger_show failed with status 5 \(TooLarge\)/);
      return true;
    });
  } finally {
    lib.closeSession(handle);
  }
});

test("an empty request is the empty message", () => {
  const lib = library();
  const handle = lib.openSession(new api.SessionConfig().encode());
  try {
    const shown = api.Outcome.decode(lib.call("ledger_show", handle, new Uint8Array(0)));
    assert.equal(shown.whichResult(), "ledgerShow");
  } finally {
    lib.closeSession(handle);
  }
});

test("a response is copied out of the library's buffer, which is freed each time", () => {
  // A loop of calls whose buffers are all released: if the free were wrong the process would
  // crash or the bytes of an earlier response would show in a later one.
  const lib = library();
  const handle = lib.openSession(new api.SessionConfig().encode());
  try {
    const request = new api.LedgerShowRequest().encode();
    const first = lib.call("ledger_show", handle, request);
    for (let i = 0; i < 200; i += 1) {
      assert.deepEqual(lib.call("ledger_show", handle, request), first);
    }
  } finally {
    lib.closeSession(handle);
  }
});

test("the library the tests build honours a fixed clock", () =>
  withGolden("leased.ucdk", async (golden) => {
    // `cargo xtask sdk-test` builds the library with the `test-clock` feature, so a session whose
    // clock is fixed shows a lease by it (see scenarios.test.ts for every state).
    await using session = await openSession({
      ledgerKind: "file",
      ledgerLocation: golden.path,
      testClockMs: WHILE_HELD_MS,
    });
    assert.equal((await session.ledgerShow()).ledgerShow?.lease?.state, api.LeaseState.LIVE);
  }));

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
    const error = errorFor("ucdk_ledger_show", status, new Uint8Array(0));
    assert.equal(error.constructor, cls, String(status));
    assert.equal(error.status, status);
    assert.ok(error instanceof UcdkError);
    assert.ok(error instanceof Error);
    assert.equal(error.name, cls.name);
    assert.match(error.message, new RegExp(`^ucdk_ledger_show failed with status ${status} `));
  }
});

test("a status the sdk does not know is still an error carrying the number", () => {
  const error = errorFor("ucdk_ledger_show", 77, new Uint8Array(0));
  assert.equal(error.constructor, UcdkError);
  assert.equal(error.status, 77);
});

test("the message of the library is kept", () => {
  const detail = new api.Error({ code: api.ErrorCode.LEDGER, message: "the ledger is held" });
  const error = errorFor("ucdk_session_open", 101, detail.encode());
  assert.equal(error.error?.message, "the ledger is held");
  assert.ok(error.message.endsWith(": the ledger is held"));
});

test("detail that is not an error message is dropped, not thrown", () => {
  const error = errorFor("ucdk_ledger_show", 101, Uint8Array.of(0x0a, 0xff));
  assert.equal(error.error, undefined);
  assert.equal(error.constructor, Internal);
});

test("a detail with no message adds nothing", () => {
  const detail = new api.Error({ code: api.ErrorCode.LEDGER });
  const error = errorFor("ucdk_ledger_show", 3, detail.encode());
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
  for (const name of ["api", "errors", "SDK_ABI"] as const) {
    assert.ok(name in sdk, name);
  }
});
