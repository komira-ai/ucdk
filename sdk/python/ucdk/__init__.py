# SPDX-License-Identifier: Apache-2.0
"""The ucdk Python SDK: a session over the ucdk library.

    from ucdk import Session, api

    config = api.SessionConfig(ledger_kind="file", ledger_location="ledger.ucdk")
    with Session(config, library_path="libucdk_capi.so") as session:
        shown = session.ledger_show()
        print(shown.ledger_show.token, shown.ledger_show.fence)

Everything that crosses to the library is protobuf bytes, written and read by the generated
classes in `ucdk.generated`; this package has no protobuf dependency.
"""

from . import errors
from ._library import Library
from .errors import UcdkError
from .generated import ucdk_api_v1 as api
from .session import Session

__all__ = ["Library", "Session", "UcdkError", "api", "errors"]
