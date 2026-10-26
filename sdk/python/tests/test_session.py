# SPDX-License-Identifier: Apache-2.0
"""The session over the real library: closing, the errors the library answers with, and the
buffers that cross the boundary."""

import unittest

from ucdk import Library, Session, api, errors
from ucdk.errors import (
    Busy,
    DecodeFailed,
    InvalidArgument,
    Panic,
    SessionClosed,
    TooLarge,
    UcdkError,
    Unsupported,
    error_for,
)

from .support import (
    WHILE_HELD_MS,
    GoldenCopy,
    library,
    library_path,
    open_file_session,
    open_session,
)


class Closing(unittest.TestCase):
    def test_a_closed_session_refuses_its_verb(self):
        session = open_session()
        self.assertFalse(session.closed)
        session.close()
        self.assertTrue(session.closed)
        with self.assertRaises(SessionClosed):
            session.ledger_show()

    def test_closing_twice_is_not_an_error(self):
        session = open_session()
        session.close()
        session.close()

    def test_a_session_is_a_context_manager_that_closes(self):
        with open_session() as session:
            self.assertFalse(session.closed)
        self.assertTrue(session.closed)

    def test_the_library_refuses_a_handle_that_was_closed(self):
        lib = library()
        handle = lib.open_session(api.SessionConfig().encode())
        lib.close_session(handle)
        with self.assertRaises(InvalidArgument) as caught:
            lib.call("ledger_show", handle, api.LedgerShowRequest().encode())
        self.assertEqual(caught.exception.status, 1)
        # Closing a handle again, or one never opened, is ignored.
        lib.close_session(handle)
        lib.close_session(12345)

    def test_a_sessions_calls_do_not_reach_another_session(self):
        with GoldenCopy("records.ucdk") as golden:
            with open_file_session(golden.path) as first, open_session() as second:
                self.assertEqual(len(first.ledger_show().ledger_show.records), 5)
                self.assertEqual(second.ledger_show().ledger_show.records, [])

    def test_a_session_whose_open_failed_can_be_collected(self):
        with self.assertRaises(Unsupported):
            Session(api.SessionConfig(ledger_kind="tape"), library_path=library_path())

    def test_a_session_can_share_a_library_that_is_already_loaded(self):
        lib = library()
        with Session(api.SessionConfig(), library=lib) as one:
            with Session(api.SessionConfig(), library=lib) as two:
                self.assertEqual(one.ledger_show(), two.ledger_show())


class Statuses(unittest.TestCase):
    """The library answers a call that did not work with a status code; the SDK raises."""

    def test_a_ledger_kind_this_library_does_not_have_is_unsupported(self):
        with self.assertRaises(Unsupported) as caught:
            open_session(ledger_kind="tape")
        self.assertEqual(caught.exception.status, 4)

    def test_a_file_ledger_with_no_location_is_an_invalid_argument(self):
        with self.assertRaises(InvalidArgument) as caught:
            open_session(ledger_kind="file")
        self.assertEqual(caught.exception.status, 1)

    def test_an_empty_config_opens_a_memory_ledger(self):
        with Session(api.SessionConfig(), library_path=library_path()) as session:
            self.assertEqual(session.ledger_show().ledger_show.token, "v0")

    def test_bytes_that_are_not_the_message_are_a_decode_error(self):
        lib = library()
        handle = lib.open_session(api.SessionConfig().encode())
        try:
            with self.assertRaises(DecodeFailed) as caught:
                lib.call("ledger_show", handle, b"\x0a\xff\xff")
            self.assertEqual(caught.exception.status, 2)
            self.assertTrue(caught.exception.error.message)
        finally:
            lib.close_session(handle)

    def test_a_request_over_the_size_limit_is_too_large(self):
        lib = library()
        handle = lib.open_session(api.SessionConfig().encode())
        try:
            with self.assertRaises(TooLarge) as caught:
                lib.call("ledger_show", handle, bytes(64 * 1024 * 1024 + 1))
            self.assertEqual(caught.exception.status, 5)
        finally:
            lib.close_session(handle)

    def test_an_empty_request_is_the_empty_message(self):
        lib = library()
        handle = lib.open_session(api.SessionConfig().encode())
        try:
            shown = api.Outcome.decode(lib.call("ledger_show", handle, b""))
        finally:
            lib.close_session(handle)
        self.assertEqual(shown.which_result(), "ledger_show")

    def test_the_library_the_tests_build_honours_a_fixed_clock(self):
        # `cargo xtask sdk-test` builds the library with the `test-clock` feature, so a session
        # whose clock is fixed shows a lease by it (see test_scenarios for every state).
        with GoldenCopy("leased.ucdk") as golden:
            with open_file_session(golden.path, test_clock_ms=WHILE_HELD_MS) as session:
                lease = session.ledger_show().ledger_show.lease
        self.assertEqual(lease.state, api.LeaseState.LIVE)


class Mapping(unittest.TestCase):
    """The table from a status code to an exception, without the library."""

    def test_each_status_has_its_class(self):
        for status, cls in [
            (1, InvalidArgument),
            (2, DecodeFailed),
            (3, Busy),
            (4, Unsupported),
            (5, TooLarge),
            (100, Panic),
            (101, errors.Internal),
        ]:
            with self.subTest(status):
                error = error_for("ucdk_ledger_show", status, b"")
                self.assertIs(type(error), cls)
                self.assertEqual(error.status, status)
                self.assertIsInstance(error, UcdkError)
                self.assertIn("ucdk_ledger_show failed with status %d" % status, str(error))

    def test_a_status_the_sdk_does_not_know_is_still_an_error_carrying_the_number(self):
        error = error_for("ucdk_ledger_show", 77, b"")
        self.assertIs(type(error), UcdkError)
        self.assertEqual(error.status, 77)

    def test_the_message_of_the_library_is_kept(self):
        detail = api.Error(code=api.ErrorCode.LEDGER, message="the ledger is held").encode()
        error = error_for("ucdk_session_open", 101, detail)
        self.assertEqual(error.error.message, "the ledger is held")
        self.assertTrue(str(error).endswith(": the ledger is held"))

    def test_detail_that_is_not_an_error_message_is_dropped_not_raised(self):
        error = error_for("ucdk_ledger_show", 101, b"\x0a\xff")
        self.assertIsNone(error.error)
        self.assertIs(type(error), errors.Internal)

    def test_a_detail_with_no_message_adds_nothing(self):
        error = error_for("ucdk_ledger_show", 3, api.Error(code=api.ErrorCode.LEDGER).encode())
        self.assertTrue(str(error).endswith("(Busy)"))


class Exports(unittest.TestCase):
    def test_the_package_names_what_a_user_needs(self):
        import ucdk

        self.assertIs(ucdk.Library, Library)
        self.assertIs(ucdk.Session, Session)
        self.assertIs(ucdk.UcdkError, UcdkError)
        for name in ucdk.__all__:
            self.assertTrue(hasattr(ucdk, name), name)


if __name__ == "__main__":
    unittest.main()
