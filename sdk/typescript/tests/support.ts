// SPDX-License-Identifier: Apache-2.0
/**
 * What the SDK tests share: the library, the ledger goldens and the clocks a session reads.
 */

import { copyFileSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { Library, Session, api } from "../src/index.ts";

export const REPO = fileURLToPath(new URL("../../../", import.meta.url));
export const LEDGER_GOLDEN = join(REPO, "sdk/ledger-golden");
// The clocks a session reads, in milliseconds, against the lease in `leased.ucdk`: `ci-7` took it
// at 1_000_000 for an hour, so it lapses at 4_600_000, and another holder is refused until the
// skew bound (5_000) after that. The tests never read the time.
export const WHILE_HELD_MS = 2_000_000n;
export const EXPIRES_MS = 4_600_000n;
export const LAPSING_MS = 4_602_000n;
export const AFTER_LAPSE_MS = 10_000_000n;
export const LIBRARY = join(
  REPO,
  "target/debug",
  process.platform === "darwin" ? "libucdk_capi.dylib" : "libucdk_capi.so",
);

/** The library `cargo xtask sdk-test` builds. */
export function libraryPath(): string {
  if (!existsSync(LIBRARY)) {
    throw new Error(`${LIBRARY} is not built; run \`cargo xtask sdk-test\`, which builds it`);
  }
  return LIBRARY;
}

export function library(): Library {
  return new Library(libraryPath());
}

/** A session on a memory ledger unless the config says otherwise. */
export async function openSession(config: api.SessionConfigInit = {}): Promise<Session> {
  return Session.open(new api.SessionConfig({ ledgerKind: "memory", ...config }), {
    libraryPath: libraryPath(),
  });
}

/** A copy of a ledger golden in a directory of its own, so a session can never change the golden. */
export interface GoldenCopy {
  /** The ledger file. */
  path: string;
  /** The directory's files with their bytes, by name. */
  listing(): Record<string, string>;
}

/** Runs `body` with a copy of the ledger golden `name`, and removes the copy afterwards. */
export async function withGolden<T>(name: string, body: (golden: GoldenCopy) => Promise<T>): Promise<T> {
  const directory = mkdtempSync(join(tmpdir(), "ucdk-golden-"));
  const path = join(directory, "ledger.ucdk");
  copyFileSync(join(LEDGER_GOLDEN, name), path);
  try {
    return await body({
      path,
      listing: () =>
        Object.fromEntries(
          readdirSync(directory)
            .sort()
            .map((file) => [file, readFileSync(join(directory, file)).toString("latin1")]),
        ),
    });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

/** The bytes of a message as a `Buffer`, which `assert` compares with the bytes of a file. */
export function bytesOf(message: { encode(): Uint8Array }): Buffer {
  return Buffer.from(message.encode());
}
