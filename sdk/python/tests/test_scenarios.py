# SPDX-License-Identifier: Apache-2.0
"""`ledger_show` through the SDK, over the real library and the ledger goldens of
`sdk/ledger-golden`: the document the library shows, the lease by the session's clock, and the
ledger errors, as data."""

import tempfile
import unittest
from pathlib import Path

from ucdk import api

from .support import (
    AFTER_LAPSE_MS,
    EXPIRES_MS,
    LAPSING_MS,
    WHILE_HELD_MS,
    GoldenCopy,
    open_file_session,
    open_session,
)


class Document(unittest.TestCase):
    def test_a_ledger_show_over_the_records_golden_gives_its_token_fence_and_records(self):
        with GoldenCopy("records.ucdk") as golden, open_file_session(golden.path) as session:
            shown = session.ledger_show()
        self.assertEqual(shown.which_result(), "ledger_show")
        self.assertEqual(shown.exit, api.ExitClass.DONE)
        document = shown.ledger_show
        self.assertEqual((document.token, document.fence), ("v5", 0))
        self.assertIsNone(document.lease)
        self.assertEqual(
            [r.path for r in document.records],
            ["app/db", "app/old", "app/pending", "app/run", "shared/found"],
        )
        run = document.records[3]
        self.assertEqual((run.kind, run.name, run.physical), ("service", "app-run", "run-1"))
        self.assertEqual(run.deps, ["app/db"])
        self.assertEqual([(o.key, o.value) for o in run.outputs], [("URL", "run-url")])
        self.assertEqual(
            [(r.origin, r.state) for r in document.records],
            [
                (api.RecordOrigin.CREATED, api.RecordState.CONFIRMED),
                (api.RecordOrigin.CREATED, api.RecordState.REAPING),
                (api.RecordOrigin.CREATED, api.RecordState.INTENT),
                (api.RecordOrigin.CREATED, api.RecordState.CONFIRMED),
                (api.RecordOrigin.ADOPTED, api.RecordState.CONFIRMED),
            ],
        )
        found = document.records[4]
        self.assertEqual([(o.key, o.value) for o in found.outputs], [("EMPTY", ""), ("NOTE", "two words")])

    def test_a_ledger_show_of_a_memory_ledger_is_the_empty_document(self):
        with open_session() as session:
            document = session.ledger_show().ledger_show
        self.assertEqual((document.token, document.fence, document.records), ("v0", 0, []))
        self.assertIsNone(document.lease)

    def test_a_ledger_show_takes_no_lease_and_changes_no_file(self):
        with GoldenCopy("leased.ucdk") as golden:
            before = golden.listing()
            with open_file_session(golden.path, test_clock_ms=WHILE_HELD_MS) as session:
                for _ in range(2):
                    document = session.ledger_show().ledger_show
                    self.assertEqual((document.fence, document.lease.holder), (1, "ci-7"))
            self.assertEqual(golden.listing(), before)


class Lease(unittest.TestCase):
    def shown_lease(self, now_ms, holder=""):
        with GoldenCopy("leased.ucdk") as golden:
            with open_file_session(golden.path, test_clock_ms=now_ms, holder=holder) as session:
                return session.ledger_show().ledger_show.lease

    def test_another_holders_lease_is_live_then_lapsing_then_lapsed_by_the_session_clock(self):
        for now_ms, state in [
            (WHILE_HELD_MS, api.LeaseState.LIVE),
            (EXPIRES_MS - 1, api.LeaseState.LIVE),
            (EXPIRES_MS, api.LeaseState.LAPSING),
            (LAPSING_MS, api.LeaseState.LAPSING),
            (AFTER_LAPSE_MS, api.LeaseState.LAPSED),
        ]:
            with self.subTest(now_ms):
                lease = self.shown_lease(now_ms, holder="ci-8")
                self.assertEqual(lease.state, state)
                self.assertEqual((lease.holder, lease.expires_ms), ("ci-7", EXPIRES_MS))
                self.assertFalse(lease.yours)

    def test_a_session_named_as_the_holder_sees_the_lease_as_its_own(self):
        for now_ms in (WHILE_HELD_MS, LAPSING_MS, AFTER_LAPSE_MS):
            with self.subTest(now_ms):
                self.assertTrue(self.shown_lease(now_ms, holder="ci-7").yours)
                self.assertFalse(self.shown_lease(now_ms, holder="ci-8").yours)
                self.assertFalse(self.shown_lease(now_ms).yours)


class LedgerErrors(unittest.TestCase):
    """A ledger that cannot be read is data: an outcome with exit class FAILED_NO_EFFECT."""

    def test_a_file_that_is_not_a_ledger_is_the_ledger_error(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "ledger.ucdk"
            path.write_text("this is not a ledger\n")
            with open_file_session(path) as session:
                shown = session.ledger_show()
            self.assertEqual(path.read_text(), "this is not a ledger\n")
        self.assertEqual(shown.which_result(), "error")
        self.assertEqual(shown.exit, api.ExitClass.FAILED_NO_EFFECT)
        self.assertEqual(shown.error.code, api.ErrorCode.LEDGER)
        self.assertTrue(shown.error.message)

    def test_a_missing_file_is_the_ledger_error_and_no_file_is_made(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "ledger.ucdk"
            with open_file_session(path) as session:
                shown = session.ledger_show()
            self.assertEqual(list(Path(directory).iterdir()), [])
        self.assertEqual(shown.error.code, api.ErrorCode.LEDGER)
        self.assertIn("does not exist", shown.error.message)


if __name__ == "__main__":
    unittest.main()
