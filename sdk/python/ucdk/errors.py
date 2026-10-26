# SPDX-License-Identifier: Apache-2.0
"""What can go wrong in a call, as exceptions.

A ledger that cannot be read is not an exception. It comes back as data in the `Outcome`
(`outcome.error`, `outcome.exit`). An exception here means the call itself did not work: the
library could not be loaded, it is the wrong version, the session is closed, or the library
answered with a non-zero status code.
"""

from typing import Dict, Optional, Type

from .generated import _wire
from .generated import ucdk_api_v1 as api


class UcdkError(Exception):
    """The base of every error the SDK raises."""

    #: The status code of the library, or 0 for an error that did not come from a call.
    status = 0

    def __init__(self, message: str, error: Optional[api.Error] = None) -> None:
        super().__init__(message)
        #: The `Error` message the library sent with the status, when it sent one.
        self.error = error


class LibraryError(UcdkError):
    """The shared library could not be loaded, or is not a ucdk library."""


class AbiMismatch(LibraryError):
    """The library's ABI version is not one this SDK can use."""


class SessionClosed(UcdkError):
    """The session was closed."""


class InvalidArgument(UcdkError):
    """UCDK_INVALID_ARGUMENT: null where not allowed, a bad length, or a handle that is not open."""

    status = 1


class DecodeFailed(UcdkError):
    """UCDK_DECODE: the request bytes are not the expected message."""

    status = 2


class Busy(UcdkError):
    """UCDK_BUSY: the session is in use by another call."""

    status = 3


class Unsupported(UcdkError):
    """UCDK_ABI: the session config asks for something this library does not have."""

    status = 4


class TooLarge(UcdkError):
    """UCDK_TOO_LARGE: the request is over the library's size limit."""

    status = 5


class Panic(UcdkError):
    """UCDK_PANIC: an internal panic was contained; the session is poisoned and can only be closed."""

    status = 100


class Internal(UcdkError):
    """UCDK_INTERNAL: an invariant the library checks failed."""

    status = 101


_BY_STATUS: Dict[int, Type[UcdkError]] = {
    cls.status: cls
    for cls in (InvalidArgument, DecodeFailed, Busy, Unsupported, TooLarge, Panic, Internal)
}


def error_for(call: str, status: int, detail: bytes) -> UcdkError:
    """The exception for a non-zero `status` of `call`; `detail` is the bytes of the `Error`
    message the library filled in, when it did."""
    error: Optional[api.Error] = None
    if detail:
        try:
            error = api.Error.decode(detail)
        except _wire.DecodeError:
            error = None
    cls = _BY_STATUS.get(status, UcdkError)
    message = "%s failed with status %d (%s)" % (call, status, cls.__name__)
    if error is not None and error.message:
        message += ": " + error.message
    exception = cls(message, error)
    exception.status = status
    return exception
