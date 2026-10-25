// SPDX-License-Identifier: Apache-2.0
/**
 * The C ABI of the ucdk library (`crates/ucdk-capi/include/ucdk.h`), bound with `node:ffi`.
 *
 * This is the whole of the binding: load the library, check its version, call the nine
 * functions. Nothing is read from the environment; the library is named by an argument.
 *
 * A `Library` calls the library on the thread it is used on, and a call blocks that thread until
 * the engine returns. `Session` runs a `Library` on a worker thread and exposes promises.
 */

import type * as Ffi from "node:ffi";

import { AbiMismatch, LibraryError, errorFor } from "./errors.ts";

/** The ABI this SDK was written for, as [major, minor]: the header's UCDK_ABI_MAJOR/MINOR. */
export const SDK_ABI: readonly [number, number] = [0, 1];

/** The verbs that take a request message and answer with a response message. */
export type Verb = "load_catalog" | "plan" | "apply" | "destroy" | "status";

const VERBS: readonly Verb[] = ["load_catalog", "plan", "apply", "destroy", "status"];

/** [major, minor] of `ucdk_abi_version()`, which is (major << 16) | minor. */
export function splitVersion(version: number): [number, number] {
  return [version >>> 16, version & 0xffff];
}

/** An SDK built for [M, m] accepts a library with the same M and a minor of at least m. */
export function accepts(sdk: readonly [number, number], library: readonly [number, number]): boolean {
  return library[0] === sdk[0] && library[1] >= sdk[1];
}

export interface LibraryOptions {
  /** The ABI the SDK needs; a test may raise or lower it to see a library refused. */
  sdkAbi?: readonly [number, number];
}

/** What a call answered: the status code, and the bytes the library filled in. */
export interface Answer {
  status: number;
  data: Uint8Array;
}

/** What opening a session answered; `handle` is 0 unless the status is 0. */
export interface Opened extends Answer {
  handle: bigint;
}

type Call = (...args: unknown[]) => unknown;

/** A loaded ucdk library, checked for its version. */
export class Library {
  readonly path: string;
  /** The library's ABI version, [major, minor], checked before any other call. */
  readonly abi: readonly [number, number];

  readonly #ffi: typeof Ffi;
  readonly #open: Call;
  readonly #close: Call;
  readonly #free: Call;
  readonly #verbs: ReadonlyMap<Verb, Call>;

  /** Loads the library at `libraryPath` and refuses it unless its ABI version is one the SDK can use. */
  constructor(libraryPath: string, options: LibraryOptions = {}) {
    const ffi = process.getBuiltinModule("node:ffi") as typeof Ffi | undefined;
    if (ffi === undefined) {
      throw new LibraryError("this Node has no node:ffi; the ucdk SDK needs Node 26.12 or newer");
    }
    let library: Ffi.DynamicLibrary;
    try {
      library = new ffi.DynamicLibrary(libraryPath);
    } catch (error) {
      throw new LibraryError(`cannot load the ucdk library ${libraryPath}: ${describe(error)}`, undefined);
    }
    this.path = libraryPath;
    this.#ffi = ffi;
    this.abi = this.#checkVersion(library, options.sdkAbi ?? SDK_ABI);
    const fn = (name: string, args: string[], ret: string): Call =>
      library.getFunction(name, { arguments: args, return: ret }) as Call;
    // `ucdk_buf_free` takes the 16 byte `ucdk_buf` by value. The C calling conventions of Linux
    // x86-64 and of arm64 (Linux and macOS) pass a two word struct of a pointer and a size_t
    // in the registers of two integer arguments, so it is declared as those two arguments.
    // `node:ffi` has no struct types.
    this.#free = fn("ucdk_buf_free", ["pointer", "uint64"], "void");
    this.#open = fn("ucdk_session_open", ["buffer", "uint64", "buffer", "buffer"], "int32");
    this.#close = fn("ucdk_session_close", ["pointer"], "void");
    this.#verbs = new Map(
      VERBS.map((verb) => [verb, fn(`ucdk_${verb}`, ["pointer", "buffer", "uint64", "buffer"], "int32")]),
    );
  }

  #checkVersion(library: Ffi.DynamicLibrary, sdk: readonly [number, number]): readonly [number, number] {
    let version: Call;
    try {
      version = library.getFunction("ucdk_abi_version", { arguments: [], return: "uint32" }) as Call;
    } catch (error) {
      throw new LibraryError(`${this.path} does not export ucdk_abi_version: ${describe(error)}`);
    }
    const abi = splitVersion(version() as number);
    if (!accepts(sdk, abi)) {
      throw new AbiMismatch(
        `this SDK needs ucdk ABI ${sdk[0]}.${sdk[1]} or a later ${sdk[0]}.x; ${this.path} is ABI ${abi[0]}.${abi[1]}`,
      );
    }
    return abi;
  }

  /** The bytes of a `ucdk_buf` the library filled, which are then released. */
  #take(buf: Buffer): Uint8Array {
    const pointer = buf.readBigUInt64LE(0);
    const length = buf.readBigUInt64LE(8);
    if (pointer === 0n) {
      return new Uint8Array(0);
    }
    const data = Uint8Array.from(this.#ffi.toBuffer(pointer, Number(length), true));
    this.#free(pointer, length);
    return data;
  }

  /** Opens a session from a `SessionConfig`; the answer holds the handle, or the status and the error. */
  tryOpen(config: Uint8Array): Opened {
    const handle = Buffer.alloc(8);
    const error = Buffer.alloc(16);
    const status = this.#open(config, config.length, handle, error) as number;
    // A handle is a number the library hands out; zero is never one.
    return { status, data: this.#take(error), handle: handle.readBigUInt64LE(0) };
  }

  /** Opens a session from a `SessionConfig`; returns its handle, or throws the status as an error. */
  openSession(config: Uint8Array): bigint {
    const { status, data, handle } = this.tryOpen(config);
    if (status !== 0) {
      throw errorFor("ucdk_session_open", status, data);
    }
    return handle;
  }

  closeSession(handle: bigint): void {
    this.#close(handle);
  }

  /** Calls a verb with a request message; returns the status and the bytes it filled in. */
  answer(verb: Verb, handle: bigint, request: Uint8Array): Answer {
    const out = Buffer.alloc(16);
    const status = (this.#verbs.get(verb) as Call)(handle, request, request.length, out) as number;
    return { status, data: this.#take(out) };
  }

  /** Calls a verb with a request message; returns the response message's bytes. */
  call(verb: Verb, handle: bigint, request: Uint8Array): Uint8Array {
    const { status, data } = this.answer(verb, handle, request);
    if (status !== 0) {
      throw errorFor(`ucdk_${verb}`, status, data);
    }
    return data;
  }
}

/** The message of what `node:ffi` threw, which is always an `Error`. */
function describe(error: unknown): string {
  return (error as Error).message;
}
