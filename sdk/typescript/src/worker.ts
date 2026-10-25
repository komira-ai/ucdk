// SPDX-License-Identifier: Apache-2.0
/**
 * The worker thread of a session: it loads the library, then runs the calls it is sent one at a
 * time, so a call that blocks until the engine returns blocks this thread and not the caller's.
 */

import { ok } from "node:assert";
import { parentPort, workerData } from "node:worker_threads";

import { AbiMismatch } from "./errors.ts";
import { Library } from "./library.ts";
import type { Ready, Reply, Request } from "./protocol.ts";

ok(parentPort !== null, "worker.ts is the entry point of a worker thread");
const port = parentPort;

/** The library, or why it could not be loaded: `Library` throws only `UcdkError`s. */
function load(libraryPath: string): Library | Ready {
  try {
    return new Library(libraryPath);
  } catch (error) {
    const kind = error instanceof AbiMismatch ? "abi" : "library";
    return { ready: false, kind, message: (error as Error).message };
  }
}

const loaded = load((workerData as { libraryPath: string }).libraryPath);
if (!(loaded instanceof Library)) {
  port.postMessage(loaded);
} else {
  const library = loaded;
  port.postMessage({ ready: true } satisfies Ready);
  port.on("message", (request: Request) => {
    let reply: Reply;
    switch (request.op) {
      case "open": {
        reply = { id: request.id, ...library.tryOpen(request.config) };
        break;
      }
      case "close":
        library.closeSession(request.handle);
        reply = { id: request.id, status: 0, data: new Uint8Array(0), handle: 0n };
        break;
      case "call":
        reply = { id: request.id, handle: 0n, ...library.answer(request.verb, request.handle, request.request) };
        break;
    }
    port.postMessage(reply);
  });
}
