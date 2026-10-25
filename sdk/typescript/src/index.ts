// SPDX-License-Identifier: Apache-2.0
/**
 * The ucdk TypeScript SDK: typed resources, and a session over the ucdk library.
 *
 *     import { Session, api, proto, resources } from "ucdk";
 *
 *     const web = new resources.Service("web", proto.Image.digest("sha256:..."));
 *     await using session = await Session.open(
 *       new api.SessionConfig({ provider: "fake", ledgerKind: "memory" }),
 *       { libraryPath: "libucdk_capi.so" },
 *     );
 *     const plan = await session.plan(resources.resourceList(web));
 *
 * Everything that crosses to the library is protobuf bytes, written and read by the generated
 * classes in `generated`; this package has no protobuf dependency. It needs the `node:ffi` of
 * Node 26.
 */

export * as errors from "./errors.ts";
export { UcdkError } from "./errors.ts";
export * as api from "./generated/ucdk_api_v1.ts";
export * as proto from "./generated/ucdk_resource_v1.ts";
export * as resources from "./generated/resources.ts";
export { Library, SDK_ABI } from "./library.ts";
export { Session } from "./session.ts";
export type { SessionOptions } from "./session.ts";
