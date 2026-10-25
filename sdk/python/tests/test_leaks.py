# SPDX-License-Identifier: Apache-2.0
"""The leak sweep through the real library: an object that carries the cell's stamp and has no
ledger record is reported with exit class LEAKS (8), on both ledgers; nothing is changed or
deleted; an apply reports leaks and is still done."""

import os
import tempfile
import unittest

from ucdk import api

from .support import open_session, pair


def sessions():
    """A session on each ledger, with the provider that holds one stamped, unrecorded object."""
    with open_session("fake-leaky") as memory:
        yield "memory", memory
    with tempfile.TemporaryDirectory() as directory:
        location = os.path.join(directory, "ledger")
        with open_session("fake-leaky", ledger_kind="file", ledger_location=location) as file:
            yield "file", file


class Leaks(unittest.TestCase):
    def test_a_stamped_object_with_no_record_is_a_leak_with_exit_8(self):
        # Proves: the verb reaches the library, the response decodes with the leaks arm, and
        # the exit class is 8, on the memory and the file ledger. Catches a verb bound to the
        # wrong symbol, or a class mapped to done.
        for name, session in sessions():
            with self.subTest(name):
                outcome = session.leaks()
                self.assertEqual(outcome.which_result(), "leaks")
                self.assertEqual(outcome.exit, api.ExitClass.LEAKS)
                self.assertEqual(int(outcome.exit), 8)
                [leak] = outcome.leaks.leaks
                self.assertEqual((leak.kind, leak.name, leak.owner), ("service", "ghost-run", "ghost"))
                self.assertNotEqual(leak.physical_id, "")
                self.assertNotEqual(leak.node, "")

    def test_a_world_with_no_leak_is_done(self):
        # Proves: the same sweep over the ordinary fake is exit 0 with an empty list. Catches a
        # sweep that reports recorded or absent objects.
        with open_session("fake") as session:
            outcome = session.leaks()
            self.assertEqual(outcome.exit, api.ExitClass.DONE)
            self.assertEqual(outcome.leaks.leaks, [])
            session.apply(pair(), session.plan(pair()).planned.hash)
            self.assertEqual(session.leaks().exit, api.ExitClass.DONE)

    def test_the_sweep_changes_nothing(self):
        # Proves: sweeping twice gives the same answer, and the plan of the same list, which
        # reads the world and the ledger, is the same before and after (the leaked object is
        # still there, nothing was recorded or deleted). Catches a sweep that deletes or adopts.
        for name, session in sessions():
            with self.subTest(name):
                before = session.plan(pair()).planned.hash
                first = session.leaks()
                self.assertEqual(session.leaks().encode(), first.encode())
                self.assertEqual(session.plan(pair()).planned.hash, before)
                self.assertEqual(len(session.leaks().leaks.leaks), 1)

    def test_an_apply_reports_the_leak_and_is_still_done(self):
        # Proves: an apply of a list beside a leaked object ends done (exit 0) with the leak in
        # its report, the same object the sweep names, and the object is still leaked after.
        # Catches an apply that fails for leaks, or a report that drops them.
        for name, session in sessions():
            with self.subTest(name):
                leaked = session.leaks().leaks.leaks[0]
                planned = session.plan(pair())
                applied = session.apply(pair(), planned.planned.hash)
                self.assertEqual(applied.which_result(), "done")
                self.assertEqual(applied.exit, api.ExitClass.DONE)
                self.assertEqual(len(applied.done.created), 3)
                self.assertEqual(applied.done.leaks, [leaked])
                self.assertIsNone(applied.done.leak_sweep_failed)
                self.assertEqual(session.leaks().leaks.leaks, [leaked])


if __name__ == "__main__":
    unittest.main()
