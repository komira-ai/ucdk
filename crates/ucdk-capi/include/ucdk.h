/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The C ABI of ucdk (libucdk.so, libucdk.dylib). Eight functions; requests and responses are
 * protobuf bytes (the ucdk.api.v1 and ucdk.resource.v1 messages); no callbacks.
 *
 * Ownership: the caller owns the request bytes and the library only borrows them for the call.
 * The library allocates every ucdk_buf it fills; release each one, once, with ucdk_buf_free,
 * never with free(). A ucdk_session lives until ucdk_session_close.
 *
 * A status code says whether the call itself worked. A plan the engine refuses or a run that
 * fails is data inside the response, with status UCDK_OK.
 *
 * One call at a time per session: a call on a busy session returns UCDK_BUSY at once. Calls on
 * different sessions are independent. A call blocks until the engine returns.
 *
 * This file is checked against the library's exports by a test; change them together.
 */
#ifndef UCDK_H
#define UCDK_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ucdk_abi_version() returns (UCDK_ABI_MAJOR << 16) | UCDK_ABI_MINOR. Adding a function or a
 * verb raises the minor; changing a signature, a status meaning or an ownership rule raises the
 * major. Before the first release the major is 0 and nothing is promised. */
#define UCDK_ABI_MAJOR 0
#define UCDK_ABI_MINOR 1

/* The longest request, in bytes, that a call accepts. */
#define UCDK_MAX_REQUEST_LEN 67108864

/* Status codes. */
#define UCDK_OK 0
#define UCDK_INVALID_ARGUMENT 1
#define UCDK_DECODE 2
#define UCDK_BUSY 3
#define UCDK_ABI 4
#define UCDK_TOO_LARGE 5
#define UCDK_PANIC 100
#define UCDK_INTERNAL 101

/* An opaque session. */
typedef struct ucdk_session ucdk_session;

/* Bytes the library allocated. A zeroed buffer is empty and freeing it does nothing. */
typedef struct ucdk_buf {
    uint8_t *ptr;
    size_t len;
} ucdk_buf;

/* The version; callable before anything else. */
uint32_t ucdk_abi_version(void);

/* Opens a session from a SessionConfig message. On UCDK_OK, *out_session is the session. On any
 * other status *out_session is NULL and *out_error holds an Error message when one could be
 * made. */
int32_t ucdk_session_open(const uint8_t *config, size_t config_len, ucdk_session **out_session,
                          ucdk_buf *out_error);

/* Closes a session. NULL, a session already closed and a session never opened are ignored.
 * After a contained panic (UCDK_PANIC) the session is poisoned and can only be closed. */
void ucdk_session_close(ucdk_session *session);

/* The verbs. The request is a message of the verb's type, req_len bytes at req (NULL with a zero
 * length is the empty message). On UCDK_OK, *out holds the response; on any other status *out is
 * zeroed or holds an Error message. */

/* req: ucdk.resource.v1.ResourceList. out: ucdk.api.v1.CatalogResponse. */
int32_t ucdk_load_catalog(ucdk_session *session, const uint8_t *req, size_t req_len, ucdk_buf *out);
/* req: ucdk.api.v1.PlanRequest. out: ucdk.api.v1.Outcome. */
int32_t ucdk_plan(ucdk_session *session, const uint8_t *req, size_t req_len, ucdk_buf *out);
/* req: ucdk.api.v1.ApplyRequest. out: ucdk.api.v1.Outcome. */
int32_t ucdk_apply(ucdk_session *session, const uint8_t *req, size_t req_len, ucdk_buf *out);
/* req: ucdk.api.v1.DestroyRequest. out: ucdk.api.v1.Outcome. */
int32_t ucdk_destroy(ucdk_session *session, const uint8_t *req, size_t req_len, ucdk_buf *out);
/* req: ucdk.api.v1.StatusRequest. out: ucdk.api.v1.Outcome. */
int32_t ucdk_status(ucdk_session *session, const uint8_t *req, size_t req_len, ucdk_buf *out);

/* Releases a buffer the library filled. Once per buffer; a zeroed buffer is a no-op. */
void ucdk_buf_free(ucdk_buf buf);

#ifdef __cplusplus
}
#endif

#endif /* UCDK_H */
