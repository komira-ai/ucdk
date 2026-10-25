# ucdk for Python

The Python SDK of ucdk: typed resources, and a session over the ucdk shared library. It is pure
Python (ctypes), with no dependency: not on a protobuf package, not on a compiled extension.

```python
from ucdk import Session, api, proto, resources

db = resources.Service("db", proto.Image.digest("sha256:..."), port=5432)
web = resources.Service(
    "web",
    proto.Image.output("build", "img"),
    env={"DB": db.url},          # a typed reference to an output of db
    uses=[db],
    exposure=resources.Service.Public(),
)
wanted = resources.resource_list(web, db)

config = api.SessionConfig(provider="fake", ledger_kind="memory")
with Session(config, library_path="target/debug/libucdk_capi.so") as session:
    plan = session.plan(wanted)                    # an Outcome; changes nothing
    done = session.apply(wanted, plan.planned.hash)  # refused unless the plan is still this one
    print(done.done.created, done.exit)
```

- The library is named by `library_path`; there is no environment variable. Without it the package
  directory (`ucdk/_lib`, where a platform wheel puts the library) is searched.
- The loader checks the library's ABI version before it calls anything else: the same major, and a
  minor at least the SDK's (`ucdk._library.SDK_ABI`, the header's `UCDK_ABI_MAJOR`/`MINOR`).
- A plan the engine refuses, a conflict and a run that fails half way are data in the `Outcome`
  (`outcome.refused`, `outcome.failed`, `outcome.exit`), never exceptions. An exception
  (`ucdk.errors`) means the call itself did not work.
- `ucdk/generated` is written by `cargo xtask gen-sdk` from the protos and is not edited by hand.
  Everything else in `ucdk` is hand-written and small: `_library.py` (the nine C functions),
  `session.py` and `errors.py`.

Tests are in `tests` and run with `cargo xtask sdk-test`, which builds the library first.
