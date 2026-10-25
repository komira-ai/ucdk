// SPDX-License-Identifier: Apache-2.0
/** What the session and its worker thread say to each other. */

import type { Verb } from "./library.ts";

/** What the worker is told to do. `id` pairs an answer with its request. */
export type Request =
  | { id: number; op: "open"; config: Uint8Array }
  | { id: number; op: "close"; handle: bigint }
  | { id: number; op: "call"; verb: Verb; handle: bigint; request: Uint8Array };

/** The first thing the worker says: it loaded the library, or why it could not. */
export type Ready =
  | { ready: true }
  | { ready: false; kind: "library" | "abi"; message: string };

/** The answer to a request: the library's status code and the bytes it filled in. */
export interface Reply {
  id: number;
  status: number;
  data: Uint8Array;
  /** The handle of a session just opened; 0 for any other reply. */
  handle: bigint;
}
