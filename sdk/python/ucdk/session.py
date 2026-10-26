# SPDX-License-Identifier: Apache-2.0
"""A session: one ledger, and the verb that shows it."""

from typing import Optional, Union
from pathlib import Path

from ._library import Library
from .errors import SessionClosed
from .generated._wire import Message
from .generated import ucdk_api_v1 as api


class Session:
    """A session opened from a `SessionConfig`.

    A session runs one call at a time; a call blocks until the library returns and releases the
    GIL while it does, so run it on a worker thread where blocking matters. A ledger that cannot
    be read is data in the returned `Outcome`; an exception is raised only when the call itself
    did not work (see `ucdk.errors`).
    """

    def __init__(
        self,
        config: api.SessionConfig,
        library_path: Union[str, Path, None] = None,
        library: Optional[Library] = None,
    ) -> None:
        self._handle: Optional[int] = None
        self._library = library if library is not None else Library(library_path)
        self._handle = self._library.open_session(config.encode())

    @property
    def closed(self) -> bool:
        return self._handle is None

    def close(self) -> None:
        """Closes the session; closing it again does nothing."""
        handle, self._handle = self._handle, None
        if handle is not None:
            self._library.close_session(handle)

    def __enter__(self) -> "Session":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def __del__(self) -> None:
        # A finalizer closes only a session nobody closed; it cannot be mid-call, since a call
        # holds a reference to the session.
        self.close()

    def _call(self, verb: str, request: Message) -> bytes:
        if self._handle is None:
            raise SessionClosed("the session is closed")
        return self._library.call(verb, self._handle, request.encode())

    def ledger_show(self) -> api.Outcome:
        """Shows the session's ledger; takes no lease and changes no file.

        `outcome.ledger_show` holds the document: its `token`, the highest `fence`, the `lease`
        (absent when none is recorded: `lease.state` is by the session's clock and `lease.yours`
        says whether it names the session's holder) and the `records` in path order. A ledger
        that cannot be opened or read (a file that is missing, corrupt or of a newer major) is
        `outcome.error` with `outcome.exit` `FAILED_NO_EFFECT`.
        """
        return api.Outcome.decode(self._call("ledger_show", api.LedgerShowRequest()))
