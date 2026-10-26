# ucdk for Python

The Python SDK of ucdk: a session over the ucdk shared library. It is pure Python (ctypes), with
no dependency: not on a protobuf package, not on a compiled extension.

```python
from ucdk import Session, api

config = api.SessionConfig(ledger_kind="file", ledger_location="ledger.ucdk")
with Session(config, library_path="target/debug/libucdk_capi.so") as session:
    shown = session.ledger_show()   # an Outcome; takes no lease and changes no file
    print(shown.ledger_show.token, shown.ledger_show.fence, shown.ledger_show.lease)
```

- The library is named by `library_path`; there is no environment variable. Without it the package
  directory (`ucdk/_lib`, where a platform wheel puts the library) is searched.
- The loader checks the library's ABI version before it calls anything else: the same major and,
  while the major is 0, the same minor (a minor of major 0 may remove a function); from major 1 a
  minor at least the SDK's (`ucdk._library.SDK_ABI`, the header's `UCDK_ABI_MAJOR`/`MINOR`).
- A ledger that cannot be read is data in the `Outcome` (`outcome.error`, `outcome.exit`), never an
  exception. An exception (`ucdk.errors`) means the call itself did not work.
- `ucdk/generated` is written by `cargo xtask gen-sdk` from the protos and is not edited by hand.
  Everything else in `ucdk` is hand-written and small: `_library.py` (the five C functions),
  `session.py` and `errors.py`.

Tests are in `tests` and run with `cargo xtask sdk-test`, which builds the library first.
