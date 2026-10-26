# SPDX-License-Identifier: Apache-2.0
"""What the SDK tests share: the library, the ledger goldens and the clocks a session reads."""

import shutil
import sys
import tempfile
from pathlib import Path

from ucdk import Library, Session, api

REPO = Path(__file__).resolve().parents[3]
LEDGER_GOLDEN = REPO / "sdk/ledger-golden"
# The clocks a session reads, in milliseconds, against the lease in `leased.ucdk`: `ci-7` took it
# at 1_000_000 for an hour, so it lapses at 4_600_000, and another holder is refused until the
# skew bound (5_000) after that. The tests never read the time.
WHILE_HELD_MS = 2_000_000
EXPIRES_MS = 4_600_000
LAPSING_MS = 4_602_000
AFTER_LAPSE_MS = 10_000_000
LIBRARY = REPO / "target/debug" / (
    "libucdk_capi.dylib" if sys.platform == "darwin" else "libucdk_capi.so"
)


def library_path():
    """The library `cargo xtask sdk-test` builds."""
    if not LIBRARY.is_file():
        raise AssertionError("%s is not built; run `cargo xtask sdk-test`, which builds it" % LIBRARY)
    return LIBRARY


def library():
    return Library(library_path())


def open_session(**config):
    """A session on a memory ledger unless the config says otherwise."""
    config.setdefault("ledger_kind", "memory")
    return Session(api.SessionConfig(**config), library_path=library_path())


class GoldenCopy:
    """A copy of a ledger golden in a directory of its own, removed when the block ends, so that
    a session can never change the golden itself."""

    def __init__(self, name):
        self._name = name

    def __enter__(self):
        self._directory = tempfile.TemporaryDirectory()
        self.dir = Path(self._directory.name)
        self.path = self.dir / "ledger.ucdk"
        shutil.copy(LEDGER_GOLDEN / self._name, self.path)
        return self

    def __exit__(self, *exc):
        self._directory.cleanup()

    def listing(self):
        """The directory's files with their bytes, by name."""
        return {p.name: p.read_bytes() for p in sorted(self.dir.iterdir())}


def open_file_session(path, **config):
    return open_session(ledger_kind="file", ledger_location=str(path), **config)
