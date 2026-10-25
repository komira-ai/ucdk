# SPDX-License-Identifier: Apache-2.0
"""A session: one provider, one ledger, one cell, and the five verbs."""

from typing import Optional, Union
from pathlib import Path

from ._library import Library
from .errors import SessionClosed
from .generated._wire import Message
from .generated import ucdk_api_v1 as api
from .generated import ucdk_resource_v1 as proto


class Session:
    """A session opened from a `SessionConfig`.

    A session runs one call at a time; a call blocks until the engine returns and releases the
    GIL while it does, so run it on a worker thread where blocking matters. A refusal or a
    failed run is data in the returned `Outcome`; an exception is raised only when the call
    itself did not work (see `ucdk.errors`).
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

    def load_catalog(self, resources: proto.ResourceList) -> api.CatalogResponse:
        """Validates the list and lowers it to nodes; reads nothing and changes nothing."""
        return api.CatalogResponse.decode(self._call("load_catalog", resources))

    def plan(self, resources: proto.ResourceList, mode: api.Mode = api.Mode.APPLY) -> api.Outcome:
        """Observes the world and plans; changes nothing."""
        return api.Outcome.decode(
            self._call("plan", api.PlanRequest(resources=resources, mode=mode))
        )

    def apply(
        self, resources: proto.ResourceList, plan_hash: str, rollback: bool = False
    ) -> api.Outcome:
        """Applies the plan the approver pinned by its hash; refused if the world's plan differs."""
        return api.Outcome.decode(
            self._call(
                "apply",
                api.ApplyRequest(resources=resources, plan_hash=plan_hash, rollback=rollback),
            )
        )

    def destroy(self, resources: proto.ResourceList, plan_hash: str) -> api.Outcome:
        """Removes what the list names, under the destroy plan the approver pinned."""
        return api.Outcome.decode(
            self._call("destroy", api.DestroyRequest(resources=resources, plan_hash=plan_hash))
        )

    def status(self, resources: proto.ResourceList) -> api.Outcome:
        """Reads the ledger and the live objects; `outcome.status` holds one entry per object."""
        return api.Outcome.decode(
            self._call("status", api.StatusRequest(resources=resources))
        )
