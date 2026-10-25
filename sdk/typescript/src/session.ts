// SPDX-License-Identifier: Apache-2.0
/** A session: one provider, one ledger, one cell, and the five verbs. */

import { Worker } from "node:worker_threads";

import { AbiMismatch, LibraryError, SessionClosed, errorFor } from "./errors.ts";
import * as api from "./generated/ucdk_api_v1.ts";
import * as proto from "./generated/ucdk_resource_v1.ts";
import type { Verb } from "./library.ts";
import type { Ready, Reply, Request } from "./protocol.ts";

export interface SessionOptions {
  /** The ucdk shared library to load. There is no environment variable and no default. */
  libraryPath: string;
}

interface Waiting {
  resolve(reply: Reply): void;
  reject(error: Error): void;
}

/** A request without the id the session gives it. */
type Ask = Request extends infer R ? (R extends Request ? Omit<R, "id"> : never) : never;

/**
 * A session opened from a `SessionConfig`.
 *
 * The library is loaded on a worker thread that belongs to the session, and every call is run
 * there: a call blocks until the engine returns, and it blocks the worker, not the caller. The
 * calls of one session run one at a time, in the order they were made. A refusal or a failed run
 * is data in the returned `Outcome`; an error is thrown only when the call itself did not work
 * (see `errors`). Close the session: it holds a thread and a ledger.
 */
export class Session {
  readonly #worker: Worker;
  readonly #handle: bigint;
  readonly #waiting: Map<number, Waiting>;
  /** The id of the next request; the first, which opened the session, was 0. */
  #next = 1;
  #closed = false;

  private constructor(worker: Worker, handle: bigint, waiting: Map<number, Waiting>) {
    this.#worker = worker;
    this.#handle = handle;
    this.#waiting = waiting;
  }

  /** Loads the library, checks its version, and opens a session from `config`. */
  static async open(config: api.SessionConfig, options: SessionOptions): Promise<Session> {
    const worker = new Worker(new URL("./worker.ts", import.meta.url), {
      workerData: { libraryPath: options.libraryPath },
    });
    const waiting = new Map<number, Waiting>();
    try {
      await ready(worker);
      worker.on("message", (reply: Reply) => settle(worker, waiting, reply));
      worker.on("error", (error) => failAll(waiting, error));
      worker.on("exit", () => failAll(waiting, new SessionClosed("the session's worker thread ended")));
      const opened = await send(worker, waiting, 0, { op: "open", config: config.encode() });
      if (opened.status !== 0) {
        throw errorFor("ucdk_session_open", opened.status, opened.data);
      }
      return new Session(worker, opened.handle, waiting);
    } catch (error) {
      await worker.terminate();
      throw error;
    }
  }

  get closed(): boolean {
    return this.#closed;
  }

  /** Closes the session and ends its thread; closing it again does nothing. */
  async close(): Promise<void> {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    try {
      // Calls made before this one are ahead of it in the worker's queue, and finish first.
      await this.#ask({ op: "close", handle: this.#handle });
    } finally {
      await this.#worker.terminate();
    }
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }

  async #ask(request: Ask): Promise<Reply> {
    const id = this.#next;
    this.#next += 1;
    return send(this.#worker, this.#waiting, id, request);
  }

  async #call(verb: Verb, request: { encode(): Uint8Array }): Promise<Uint8Array> {
    if (this.#closed) {
      throw new SessionClosed("the session is closed");
    }
    const reply = await this.#ask({ op: "call", verb, handle: this.#handle, request: request.encode() });
    if (reply.status !== 0) {
      throw errorFor(`ucdk_${verb}`, reply.status, reply.data);
    }
    return reply.data;
  }

  /** Validates the list and lowers it to nodes; reads nothing and changes nothing. */
  async loadCatalog(resources: proto.ResourceList): Promise<api.CatalogResponse> {
    return api.CatalogResponse.decode(await this.#call("load_catalog", resources));
  }

  /** Observes the world and plans; changes nothing. */
  async plan(resources: proto.ResourceList, mode: api.Mode = api.Mode.APPLY): Promise<api.Outcome> {
    return api.Outcome.decode(await this.#call("plan", new api.PlanRequest({ resources, mode })));
  }

  /** Applies the plan the approver pinned by its hash; refused if the world's plan differs. */
  async apply(resources: proto.ResourceList, planHash: string, rollback = false): Promise<api.Outcome> {
    const request = new api.ApplyRequest({ resources, planHash, rollback });
    return api.Outcome.decode(await this.#call("apply", request));
  }

  /** Removes what the list names, under the destroy plan the approver pinned. */
  async destroy(resources: proto.ResourceList, planHash: string): Promise<api.Outcome> {
    const request = new api.DestroyRequest({ resources, planHash });
    return api.Outcome.decode(await this.#call("destroy", request));
  }

  /**
   * Compares the cell's stamped objects with the ledger and changes nothing. `outcome.leaks`
   * lists the objects that carry the cell's stamp and have no ledger record, and `outcome.exit`
   * is the leaks class (8) when there are any. A stamp never authorises a delete.
   */
  async leaks(): Promise<api.Outcome> {
    return api.Outcome.decode(await this.#call("leaks", new api.LeaksRequest({})));
  }

  /** Reads the ledger and the live objects; `outcome.status` holds one entry per object. */
  async status(resources: proto.ResourceList): Promise<api.Outcome> {
    return api.Outcome.decode(await this.#call("status", new api.StatusRequest({ resources })));
  }
}

/** The worker's first message: it loaded the library, or the error that says why not. */
async function ready(worker: Worker): Promise<void> {
  const first = await new Promise<Ready>((resolve, reject) => {
    worker.once("message", resolve);
    worker.once("error", reject);
    worker.once("exit", () => reject(new SessionClosed("the session's worker thread ended")));
  });
  if (!first.ready) {
    throw first.kind === "abi" ? new AbiMismatch(first.message) : new LibraryError(first.message);
  }
}

function send(worker: Worker, waiting: Map<number, Waiting>, id: number, request: Ask): Promise<Reply> {
  return new Promise<Reply>((resolve, reject) => {
    if (worker.threadId === -1) {
      reject(new SessionClosed("the session's worker thread ended"));
      return;
    }
    waiting.set(id, { resolve, reject });
    // The worker keeps the process alive only while a call is waiting for it.
    worker.ref();
    worker.postMessage({ ...request, id } as Request);
  });
}

function settle(worker: Worker, waiting: Map<number, Waiting>, reply: Reply): void {
  const entry = waiting.get(reply.id);
  waiting.delete(reply.id);
  if (waiting.size === 0) {
    worker.unref();
  }
  entry?.resolve(reply);
}

function failAll(waiting: Map<number, Waiting>, error: Error): void {
  for (const entry of waiting.values()) {
    entry.reject(error);
  }
  waiting.clear();
}
