// SPDX-License-Identifier: Apache-2.0
/**
 * What can go wrong in a call, as errors.
 *
 * A ledger that cannot be read is not an error. It comes back as data in the `Outcome`
 * (`outcome.error`, `outcome.exit`). An error here means the call itself did not work: the
 * library could not be loaded, it is the wrong version, the session is closed, or the library
 * answered with a non-zero status code.
 */

import * as wire from "./generated/wire.ts";
import * as api from "./generated/ucdk_api_v1.ts";

/** The base of every error the SDK throws. */
export class UcdkError extends Error {
  /** The status code of the library, or 0 for an error that did not come from a call. */
  status = 0;
  /** The `Error` message the library sent with the status, when it sent one. */
  readonly error: api.Error | undefined;

  constructor(message: string, error?: api.Error) {
    super(message);
    this.name = new.target.name;
    this.error = error;
  }
}

/** The shared library could not be loaded, or is not a ucdk library. */
export class LibraryError extends UcdkError {}

/** The library's ABI version is not one this SDK can use. */
export class AbiMismatch extends LibraryError {}

/** The session was closed. */
export class SessionClosed extends UcdkError {}

/** UCDK_INVALID_ARGUMENT: null where not allowed, a bad length, or a handle that is not open. */
export class InvalidArgument extends UcdkError {
  override status = 1;
}

/** UCDK_DECODE: the request bytes are not the expected message. */
export class DecodeFailed extends UcdkError {
  override status = 2;
}

/** UCDK_BUSY: the session is in use by another call. */
export class Busy extends UcdkError {
  override status = 3;
}

/** UCDK_ABI: the session config asks for something this library does not have. */
export class Unsupported extends UcdkError {
  override status = 4;
}

/** UCDK_TOO_LARGE: the request is over the library's size limit. */
export class TooLarge extends UcdkError {
  override status = 5;
}

/** UCDK_PANIC: an internal panic was contained; the session is poisoned and can only be closed. */
export class Panic extends UcdkError {
  override status = 100;
}

/** UCDK_INTERNAL: an invariant the library checks failed. */
export class Internal extends UcdkError {
  override status = 101;
}

type ErrorClass = new (message: string, error?: api.Error) => UcdkError;

const BY_STATUS = new Map<number, ErrorClass>([
  [1, InvalidArgument],
  [2, DecodeFailed],
  [3, Busy],
  [4, Unsupported],
  [5, TooLarge],
  [100, Panic],
  [101, Internal],
]);

/**
 * The error for a non-zero `status` of `call`; `detail` is the bytes of the `Error` message the
 * library filled in, when it did.
 */
export function errorFor(call: string, status: number, detail: Uint8Array): UcdkError {
  let error: api.Error | undefined;
  if (detail.length > 0) {
    try {
      error = api.Error.decode(detail);
    } catch (caught) {
      if (!(caught instanceof wire.DecodeError)) {
        throw caught;
      }
    }
  }
  const cls = BY_STATUS.get(status) ?? UcdkError;
  let message = `${call} failed with status ${status} (${cls.name})`;
  if (error !== undefined && error.message !== "") {
    message += `: ${error.message}`;
  }
  const exception = new cls(message, error);
  exception.status = status;
  return exception;
}
