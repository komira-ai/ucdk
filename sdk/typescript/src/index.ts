// SPDX-License-Identifier: Apache-2.0
/**
 * The ucdk TypeScript SDK: a session over the ucdk library.
 *
 *     import { Session, api } from "ucdk";
 *
 *     await using session = await Session.open(
 *       new api.SessionConfig({ ledgerKind: "file", ledgerLocation: "ledger.ucdk" }),
 *       { libraryPath: "libucdk_capi.so" },
 *     );
 *     const shown = await session.ledgerShow();
 *     console.log(shown.ledgerShow?.token, shown.ledgerShow?.fence);
 *
 * Everything that crosses to the library is protobuf bytes, written and read by the generated
 * classes in `generated`; this package has no protobuf dependency. It needs the `node:ffi` of
 * Node 26.
 */

export * as errors from "./errors.ts";
export { UcdkError } from "./errors.ts";
export * as api from "./generated/ucdk_api_v1.ts";
export { Library, SDK_ABI } from "./library.ts";
export { Session } from "./session.ts";
export type { SessionOptions } from "./session.ts";
