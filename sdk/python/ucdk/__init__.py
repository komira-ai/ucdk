# SPDX-License-Identifier: Apache-2.0
"""The ucdk Python SDK: typed resources, and a session over the ucdk library.

    from ucdk import Library, Session, api, proto, resources

    web = resources.Service("web", proto.Image.digest("sha256:..."))
    with Session(api.SessionConfig(provider="fake", ledger_kind="memory"),
                 library_path="libucdk_capi.so") as session:
        plan = session.plan(resources.resource_list(web))

Everything that crosses to the library is protobuf bytes, written and read by the generated
classes in `ucdk.generated`; this package has no protobuf dependency.
"""

from . import errors
from ._library import Library
from .errors import UcdkError
from .generated import resources
from .generated import ucdk_api_v1 as api
from .generated import ucdk_resource_v1 as proto
from .session import Session

__all__ = ["Library", "Session", "UcdkError", "api", "errors", "proto", "resources"]
