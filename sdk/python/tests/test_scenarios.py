# SPDX-License-Identifier: Apache-2.0
"""plan, apply, plan again, destroy and status through the SDK, over the real library and the
fake provider, held to the data the command line prints for the same scenario.

`crates/ucdk-cli/tests/scenario.rs` runs the scenario through the CLI and compares each step
with the files in `crates/ucdk-cli/tests/data`; these tests read the same files, so the SDK and
the command line must agree on the plan hash, the steps, the report and the status.
"""

import os
import shutil
import tempfile
import unittest

from ucdk import api, proto, resources
from ucdk.errors import Internal

from .support import (
    DATA,
    IMAGE,
    LEDGER_GOLDEN,
    objects_of,
    open_session,
    pair,
    read_plan,
    read_report,
    read_status,
    steps_of,
)


class Scenario(unittest.TestCase):
    def test_plan_apply_plan_again_status_destroy_give_the_command_lines_outcomes(self):
        resources_ = pair()
        with open_session() as session:
            before = session.status(resources_)
            self.assertEqual(objects_of(before), read_status("status_before_fake.txt"))
            self.assertEqual(before.exit, api.ExitClass.DONE)

            planned = session.plan(resources_)
            expected_hash, expected_steps = read_plan("plan_apply_fake.txt")
            self.assertEqual(planned.which_result(), "planned")
            self.assertEqual(planned.exit, api.ExitClass.DONE)
            self.assertEqual(planned.planned.hash, expected_hash)
            self.assertEqual(steps_of(planned.planned), expected_steps)

            applied = session.apply(resources_, planned.planned.hash)
            created = read_report("apply_fake.txt")["created"]
            self.assertEqual(applied.which_result(), "done")
            self.assertEqual(applied.exit, api.ExitClass.DONE)
            self.assertEqual(applied.done.created, created)
            self.assertEqual(
                [applied.done.updated, applied.done.deleted, applied.done.failed], [[], [], []]
            )

            self.assertEqual(objects_of(session.status(resources_)), read_status("status_fake.txt"))

            again = session.plan(resources_)
            again_hash, again_steps = read_plan("plan_again_fake.txt")
            self.assertEqual(again.planned.hash, again_hash)
            self.assertEqual(steps_of(again.planned), again_steps)

            doomed = session.plan(resources_, api.Mode.DESTROY)
            doomed_hash, doomed_steps = read_plan("plan_destroy_fake.txt")
            self.assertEqual(doomed.planned.hash, doomed_hash)
            self.assertEqual(steps_of(doomed.planned), doomed_steps)

            destroyed = session.destroy(resources_, doomed.planned.hash)
            self.assertEqual(destroyed.which_result(), "done")
            self.assertEqual(destroyed.done.deleted, read_report("destroy_fake.txt")["deleted"])

            gone = session.plan(resources_, api.Mode.DESTROY)
            gone_hash, gone_steps = read_plan("plan_destroyed_fake.txt")
            self.assertEqual((gone.planned.hash, steps_of(gone.planned)), (gone_hash, gone_steps))

    def test_the_catalog_wants_the_nodes_the_plan_creates_in_the_same_order(self):
        with open_session() as session:
            catalog = session.load_catalog(pair())
        _, steps = read_plan("plan_apply_fake.txt")
        self.assertEqual(catalog.exit, api.ExitClass.DONE)
        wanted = [(n.path, n.kind) for n in catalog.nodes if n.wanted]
        self.assertEqual(wanted, [(p, k) for _, p, k in steps])
        # A role the list turns off stays, so that its leftovers are found.
        self.assertEqual(
            [n.path for n in catalog.nodes if not n.wanted], ["example-db/public"]
        )
        deps = {n.path: n.deps for n in catalog.nodes}
        self.assertEqual(deps["example-db/run"], [])
        self.assertIn("example-db/run", deps["example-svc/run"])

    def test_a_list_the_catalog_refuses_comes_back_as_findings(self):
        dangling = resources.resource_list(
            resources.Service("web", IMAGE, uses=[proto.Ref(resource="ghost")])
        )
        with open_session() as session:
            catalog = session.load_catalog(dangling)
            planned = session.plan(dangling)
        self.assertEqual(catalog.exit, api.ExitClass.REFUSED)
        self.assertEqual(catalog.nodes, [])
        self.assertEqual([f.resource for f in catalog.findings], ["web"])
        self.assertEqual(planned.exit, api.ExitClass.REFUSED)
        self.assertEqual(planned.which_result(), "refused")
        self.assertEqual(
            [f.kind for f in planned.refused.findings.findings], [f.kind for f in catalog.findings]
        )


class Refusals(unittest.TestCase):
    """What the engine refuses is data: an outcome with exit class REFUSED, never an exception."""

    def test_a_kind_the_provider_does_not_host_is_refused_and_nothing_is_created(self):
        text = (DATA / "plan_apply_fake_limited_refused.txt").read_text()
        self.assertIn("refused: kind job is not hosted by fake-limited (example-batch/run)", text)
        wanted = resources.resource_list(
            resources.Service("example-svc", IMAGE, port=8080),
            resources.Job("example-batch", IMAGE),
        )
        with open_session("fake-limited") as session:
            outcome = session.plan(wanted)
            self.assertEqual(outcome.exit, api.ExitClass.REFUSED)
            self.assertEqual(outcome.which_result(), "refused")
            self.assertEqual(
                outcome.refused.unhosted_kinds.kinds,
                [api.UnhostedKind(path="example-batch/run", kind="job")],
            )
            applied = session.apply(wanted, "a" * 64)
            self.assertEqual(applied.exit, api.ExitClass.REFUSED)
            self.assertEqual(applied.refused.which_reason(), "unhosted_kinds")
            # A status is refused the same way; the same list without the job is hosted, and
            # nothing of it was created.
            self.assertEqual(session.status(wanted).exit, api.ExitClass.REFUSED)
            services = resources.resource_list(resources.Service("example-svc", IMAGE))
            self.assertEqual(session.plan(services).which_result(), "planned")
            self.assertEqual(
                [o.record for o in session.status(services).status.objects], [None]
            )

    def test_an_apply_under_a_hash_that_is_not_the_plan_is_refused_and_changes_nothing(self):
        wanted = pair()
        with open_session() as session:
            stale = session.apply(wanted, "a" * 64)
            real = session.plan(wanted).planned.hash
            self.assertEqual(stale.exit, api.ExitClass.REFUSED)
            self.assertEqual(stale.refused.which_reason(), "plan_hash_mismatch")
            self.assertEqual(
                stale.refused.plan_hash_mismatch,
                api.PlanHashMismatch(approved="a" * 64, plan=real),
            )
            self.assertTrue(
                all(o.record is None and o.live is None for o in session.status(wanted).status.objects)
            )

    def test_a_hash_that_is_not_a_hash_is_an_invalid_request_not_an_exception(self):
        with open_session() as session:
            outcome = session.apply(pair(), "not-a-hash")
        self.assertEqual(outcome.exit, api.ExitClass.USAGE)
        self.assertEqual(outcome.which_result(), "invalid")
        self.assertEqual(outcome.invalid.code, api.ErrorCode.INVALID_REQUEST)

    def test_a_destroy_of_nothing_is_refused_as_an_empty_scope(self):
        with open_session() as session:
            outcome = session.plan(resources.resource_list(), api.Mode.DESTROY)
            destroyed = session.destroy(resources.resource_list(), "b" * 64)
        for refused in (outcome, destroyed):
            self.assertEqual(refused.exit, api.ExitClass.REFUSED)
            self.assertEqual(refused.refused.which_reason(), "empty_scope")


class FileLedger(unittest.TestCase):
    def test_the_ledger_outlives_the_session_and_a_second_session_reads_it(self):
        wanted = pair()
        with tempfile.TemporaryDirectory() as directory:
            location = os.path.join(directory, "ledger")
            with open_session(ledger_kind="file", ledger_location=location) as first:
                plan = first.plan(wanted)
                self.assertEqual(first.apply(wanted, plan.planned.hash).which_result(), "done")
            # The fake provider's objects are in memory and went with the first session; the
            # ledger is a file, so the second session finds the records and no object.
            with open_session(ledger_kind="file", ledger_location=location) as second:
                seen = objects_of(second.status(wanted))
        self.assertEqual(
            seen,
            [
                (path, kind, api.RecordState.CONFIRMED, None, api.Standing.MISSING)
                for path, kind, _, _, _ in read_status("status_fake.txt")
            ],
        )

    def test_a_second_session_opens_beside_the_first_and_sees_what_it_writes(self):
        wanted = pair()
        with tempfile.TemporaryDirectory() as directory:
            location = os.path.join(directory, "ledger")
            with open_session(ledger_kind="file", ledger_location=location) as first:
                with open_session(ledger_kind="file", ledger_location=location) as second:
                    for entry in objects_of(second.status(wanted)):
                        self.assertEqual(entry[4], api.Standing.NOT_APPLIED)
                    plan = first.plan(wanted)
                    self.assertEqual(first.apply(wanted, plan.planned.hash).which_result(), "done")
                    seen = objects_of(second.status(wanted))
        self.assertTrue(seen)
        for _, _, record, _, standing in seen:
            self.assertEqual((record, standing), (api.RecordState.CONFIRMED, api.Standing.MISSING))

    def test_a_file_that_is_not_a_ledger_cannot_be_opened(self):
        with tempfile.TemporaryDirectory() as directory:
            location = os.path.join(directory, "ledger")
            with open(location, "w") as handle:
                handle.write("not a ledger\n")
            with self.assertRaises(Internal) as caught:
                open_session(ledger_kind="file", ledger_location=location)
        self.assertEqual(caught.exception.status, 101)
        self.assertTrue(caught.exception.error.message)

    def test_a_ledger_in_a_directory_that_is_not_there_cannot_be_opened(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(Internal):
                open_session(
                    ledger_kind="file", ledger_location=os.path.join(directory, "no", "ledger")
                )


class Lease(unittest.TestCase):
    """The ledger's lease: a run that another holder's lease excludes is refused as data."""

    def copy_of(self, directory, golden):
        location = os.path.join(directory, "ledger")
        shutil.copy(LEDGER_GOLDEN / golden, location)
        return location

    def test_an_apply_on_a_ledger_another_holder_has_leased_is_refused_as_data(self):
        wanted = pair()
        with tempfile.TemporaryDirectory() as directory:
            location = self.copy_of(directory, "leased.ucdk")
            with open_session(ledger_kind="file", ledger_location=location) as session:
                # Reading is not writing: the plan and the status are answered.
                plan = session.plan(wanted)
                self.assertEqual(plan.which_result(), "planned")
                self.assertEqual(session.status(wanted).exit, api.ExitClass.DONE)
                applied = session.apply(wanted, plan.planned.hash)
                self.assertEqual(applied.exit, api.ExitClass.REFUSED)
                self.assertEqual(applied.which_result(), "refused")
                self.assertEqual(applied.refused.which_reason(), "lease_held")
                self.assertEqual(applied.refused.lease_held.holder, "ci-7")
                self.assertGreater(applied.refused.lease_held.remaining_ms, 0)
                self.assertIn("leased to ci-7", applied.refused.message)
                # Nothing was created or recorded.
                self.assertTrue(
                    all(o.record is None and o.live is None for o in session.status(wanted).status.objects)
                )

    def test_an_apply_on_a_ledger_whose_holder_lapsed_takes_over_and_goes_through(self):
        wanted = pair()
        with tempfile.TemporaryDirectory() as directory:
            location = self.copy_of(directory, "lapsed.ucdk")
            with open_session(ledger_kind="file", ledger_location=location) as session:
                plan = session.plan(wanted)
                applied = session.apply(wanted, plan.planned.hash)
        self.assertEqual(applied.exit, api.ExitClass.DONE)
        self.assertEqual(applied.which_result(), "done")
        self.assertEqual(applied.done.created, read_report("apply_fake.txt")["created"])


if __name__ == "__main__":
    unittest.main()
