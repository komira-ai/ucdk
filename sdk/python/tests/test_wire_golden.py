# SPDX-License-Identifier: Apache-2.0
"""The wire contract with the Rust core.

`sdk/wire-golden/<case>.bin` is what the Rust messages encode to (written by
`crates/ucdk-api/tests/sdk_golden.rs`). Each case here builds the same value with the generated
classes and must (1) encode to exactly those bytes and (2) be what the bytes decode to. Building
the value by hand, and not by decoding the golden, is what makes (2) a check of the field
numbers: a decoder that read field 2 into the wrong attribute would agree with an encoder that
wrote it back the same way, and only the hand-built value tells them apart.
"""

import unittest
from pathlib import Path

from ucdk.generated import ucdk_api_v1 as api

GOLDEN = Path(__file__).resolve().parents[2] / "wire-golden"


def error(code, message):
    return api.Error(code=code, message=message)


def response(lease):
    """A document that sets every field of the response, with the lease as given."""
    return api.LedgerShowResponse(
        token="v7",
        fence=3,
        lease=lease,
        records=[
            api.LedgerRecord(
                path="app/run",
                kind="service",
                name="app-run",
                origin=api.RecordOrigin.ADOPTED,
                state=api.RecordState.REAPING,
                physical="run-1",
                deps=["app/db", "app/cache"],
                outputs=[
                    api.Pair(key="HOST", value="run-host"),
                    api.Pair(key="PORT", value="8080"),
                ],
            )
        ],
    )


def cases():
    lease = api.LeaseView(
        holder="ci-7", expires_ms=4_600_000, state=api.LeaseState.LAPSING, yours=True
    )
    return {
        "session_config": api.SessionConfig(
            ledger_kind="file",
            ledger_location="/var/ucdk/ledger",
            holder="ci-7",
            test_clock_ms=2_000_000,
        ),
        "error": error(api.ErrorCode.LEDGER, "the ledger is corrupt"),
        "outcome_invalid": api.Outcome(
            invalid=error(api.ErrorCode.INVALID_REQUEST, "the request is not valid"),
            exit=api.ExitClass.USAGE,
        ),
        "ledger_show_request": api.LedgerShowRequest(),
        "ledger_show_response": response(lease),
        "outcome_ledger_show": api.Outcome(
            ledger_show=response(None), exit=api.ExitClass.DONE
        ),
        "outcome_error": api.Outcome(
            error=error(api.ErrorCode.LEDGER, "the ledger is corrupt"),
            exit=api.ExitClass.FAILED_NO_EFFECT,
        ),
    }


def manifest():
    """{case: message name} as the Rust side wrote it."""
    out = {}
    for line in (GOLDEN / "manifest.txt").read_text().splitlines():
        file, message = line.split(" ")
        out[file[: -len(".bin")]] = message
    return out


def class_of(message_name):
    package, _, name = message_name.rpartition(".")
    module = {"ucdk.api.v1": api}[package]
    return getattr(module, name)


class WireGolden(unittest.TestCase):
    def test_every_golden_has_a_case_and_every_case_a_golden(self):
        self.assertEqual(sorted(manifest()), sorted(cases()))

    def test_each_case_is_the_message_its_manifest_line_names(self):
        for name, message in manifest().items():
            with self.subTest(name):
                self.assertIs(type(cases()[name]), class_of(message))

    def test_each_case_encodes_to_the_bytes_rust_writes(self):
        for name, value in cases().items():
            with self.subTest(name):
                self.assertEqual(value.encode(), (GOLDEN / (name + ".bin")).read_bytes())

    def test_each_golden_decodes_to_the_value_the_case_builds(self):
        for name, value in cases().items():
            with self.subTest(name):
                decoded = type(value).decode((GOLDEN / (name + ".bin")).read_bytes())
                self.assertEqual(decoded, value)

    def test_a_decoded_golden_encodes_back_to_the_same_bytes(self):
        for name, value in cases().items():
            with self.subTest(name):
                data = (GOLDEN / (name + ".bin")).read_bytes()
                self.assertEqual(type(value).decode(data).encode(), data)

    def test_a_message_with_no_fields_is_no_bytes(self):
        self.assertEqual((GOLDEN / "ledger_show_request.bin").read_bytes(), b"")
        self.assertEqual(api.LedgerShowRequest.decode(b""), api.LedgerShowRequest())

    def test_a_lease_that_is_absent_and_one_that_is_present_are_different_bytes(self):
        absent = (GOLDEN / "outcome_ledger_show.bin").read_bytes()
        present = api.Outcome(ledger_show=response(cases()["ledger_show_response"].lease)).encode()
        self.assertNotEqual(absent, present)
        self.assertIsNone(api.Outcome.decode(absent).ledger_show.lease)


if __name__ == "__main__":
    unittest.main()
