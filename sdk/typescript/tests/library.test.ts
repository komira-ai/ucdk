// SPDX-License-Identifier: Apache-2.0
/** Loading the library: where it is looked for, and which versions an SDK accepts. */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { mock, test } from "node:test";

import { AbiMismatch, LibraryError, Unsupported } from "../src/errors.ts";
import { api } from "../src/index.ts";
import { Library, SDK_ABI, accepts, splitVersion } from "../src/library.ts";
import { LIBRARY, REPO, libraryPath } from "./support.ts";

test("the library is loaded from the path given", () => {
  const lib = new Library(libraryPath());
  assert.equal(lib.path, LIBRARY);
  assert.equal(lib.abi[0], SDK_ABI[0]);
  assert.ok(lib.abi[1] >= SDK_ABI[1]);
});

test("a file that is not there is a library error naming it", () => {
  assert.throws(() => new Library("/no/such/libucdk_capi.so"), (error: unknown) => {
    assert.ok(error instanceof LibraryError);
    assert.match(error.message, /\/no\/such\/libucdk_capi\.so/);
    return true;
  });
});

test("a library that is not ucdk is refused before anything is called", () => {
  // Every Linux and macOS system has a C library to load; it exports no ucdk function.
  const libc = process.platform === "darwin" ? "/usr/lib/libSystem.B.dylib" : "libc.so.6";
  assert.throws(() => new Library(libc), (error: unknown) => {
    assert.ok(error instanceof LibraryError);
    assert.match(error.message, /ucdk_abi_version/);
    return true;
  });
});

test("a version number is major in the high half and minor in the low", () => {
  assert.deepEqual(splitVersion((3 << 16) | 7), [3, 7]);
  assert.deepEqual(splitVersion(0), [0, 0]);
  // The top bit of a 32 bit number is part of the major.
  assert.deepEqual(splitVersion(0xffff0001), [0xffff, 1]);
});

test("the same major and a minor at least the sdks is accepted", () => {
  assert.equal(accepts([1, 2], [1, 2]), true);
  assert.equal(accepts([1, 2], [1, 9]), true);
});

test("an older minor or another major is not", () => {
  assert.equal(accepts([1, 2], [1, 1]), false);
  assert.equal(accepts([1, 2], [2, 2]), false);
  assert.equal(accepts([1, 2], [0, 9]), false);
});

test("a library of another major is refused with both versions named", () => {
  const [major, minor] = new Library(libraryPath()).abi;
  assert.throws(() => new Library(libraryPath(), { sdkAbi: [major + 1, 0] }), (error: unknown) => {
    assert.ok(error instanceof AbiMismatch);
    assert.ok(error instanceof LibraryError);
    assert.match(error.message, new RegExp(`ABI ${major + 1}\\.0`));
    assert.match(error.message, new RegExp(`ABI ${major}\\.${minor}`));
    return true;
  });
});

test("a library older than the sdk needs is refused", () => {
  const [major, minor] = new Library(libraryPath()).abi;
  assert.throws(() => new Library(libraryPath(), { sdkAbi: [major, minor + 1] }), AbiMismatch);
});

test("a library newer in its minor than the sdk is accepted", () => {
  const [major, minor] = new Library(libraryPath()).abi;
  const lib = new Library(libraryPath(), { sdkAbi: [major, Math.max(minor - 1, 0)] });
  assert.deepEqual(lib.abi, [major, minor]);
});

test("the sdks abi is the headers", () => {
  const header = readFileSync(join(REPO, "crates/ucdk-capi/include/ucdk.h"), "utf8");
  const major = Number(/#define UCDK_ABI_MAJOR (\d+)/.exec(header)?.[1]);
  const minor = Number(/#define UCDK_ABI_MINOR (\d+)/.exec(header)?.[1]);
  assert.deepEqual(SDK_ABI, [major, minor]);
});

test("a node without node:ffi is told so", () => {
  const missing = mock.method(process, "getBuiltinModule", () => undefined);
  try {
    assert.throws(() => new Library(libraryPath()), (error: unknown) => {
      assert.ok(error instanceof LibraryError);
      assert.match(error.message, /no node:ffi/);
      return true;
    });
  } finally {
    missing.mock.restore();
  }
});

test("a session the library cannot open is the error of its status", () => {
  const lib = new Library(libraryPath());
  assert.throws(
    () => lib.openSession(new api.SessionConfig({ provider: "no-such-provider" }).encode()),
    (error: unknown) => {
      assert.ok(error instanceof Unsupported);
      assert.equal(error.status, 4);
      return true;
    },
  );
});
