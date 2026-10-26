// SPDX-License-Identifier: Apache-2.0
/**
 * The wire contract with the Rust core.
 *
 * `sdk/wire-golden/<case>.bin` is what the Rust messages encode to (written by
 * `crates/ucdk-api/tests/sdk_golden.rs`). Each case here builds the same value with the generated
 * classes and must (1) encode to exactly those bytes and (2) be what the bytes decode to.
 * Building the value by hand, and not by decoding the golden, is what makes (2) a check of the
 * field numbers: a decoder that read field 2 into the wrong property would agree with an encoder
 * that wrote it back the same way, and only the hand-built value tells them apart.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import * as api from "../src/generated/ucdk_api_v1.ts";
import * as wire from "../src/generated/wire.ts";
import { REPO, bytesOf } from "./support.ts";

const GOLDEN = join(REPO, "sdk/wire-golden");

type Case = wire.Message & { encode(): Uint8Array };

function error(code: api.ErrorCode, message: string): api.Error {
  return new api.Error({ code, message });
}

/** A document that sets every field of the response, with the lease as given. */
function response(lease: api.LeaseView | undefined): api.LedgerShowResponse {
  return new api.LedgerShowResponse({
    token: "v7",
    fence: 3n,
    lease,
    records: [
      new api.LedgerRecord({
        path: "app/run",
        kind: "service",
        name: "app-run",
        origin: api.RecordOrigin.ADOPTED,
        state: api.RecordState.REAPING,
        physical: "run-1",
        deps: ["app/db", "app/cache"],
        outputs: [
          new api.Pair({ key: "HOST", value: "run-host" }),
          new api.Pair({ key: "PORT", value: "8080" }),
        ],
      }),
    ],
  });
}

function cases(): Record<string, Case> {
  const lease = new api.LeaseView({
    holder: "ci-7",
    expiresMs: 4_600_000n,
    state: api.LeaseState.LAPSING,
    yours: true,
  });
  return {
    session_config: new api.SessionConfig({
      ledgerKind: "file",
      ledgerLocation: "/var/ucdk/ledger",
      holder: "ci-7",
      testClockMs: 2_000_000n,
    }),
    error: error(api.ErrorCode.LEDGER, "the ledger is corrupt"),
    outcome_invalid: new api.Outcome({
      invalid: error(api.ErrorCode.INVALID_REQUEST, "the request is not valid"),
      exit: api.ExitClass.USAGE,
    }),
    ledger_show_request: new api.LedgerShowRequest(),
    ledger_show_response: response(lease),
    outcome_ledger_show: new api.Outcome({
      ledgerShow: response(undefined),
      exit: api.ExitClass.DONE,
    }),
    outcome_error: new api.Outcome({
      error: error(api.ErrorCode.LEDGER, "the ledger is corrupt"),
      exit: api.ExitClass.FAILED_NO_EFFECT,
    }),
  };
}

/** `{case: message name}` as the Rust side wrote it. */
function manifest(): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of readFileSync(join(GOLDEN, "manifest.txt"), "utf8").split("\n")) {
    if (line !== "") {
      const [file, message] = line.split(" ") as [string, string];
      out[file.slice(0, -".bin".length)] = message;
    }
  }
  return out;
}

type Decoder = { decode(data: Uint8Array): Case };

function classOf(messageName: string): Decoder {
  const at = messageName.lastIndexOf(".");
  const modules: Record<string, Record<string, unknown>> = { "ucdk.api.v1": api };
  return (modules[messageName.slice(0, at)] as Record<string, Decoder>)[
    messageName.slice(at + 1)
  ] as Decoder;
}

function golden(name: string): Buffer {
  return readFileSync(join(GOLDEN, `${name}.bin`));
}

test("every golden has a case and every case a golden", () => {
  assert.deepEqual(Object.keys(manifest()).sort(), Object.keys(cases()).sort());
});

test("each case is the message its manifest line names", () => {
  for (const [name, message] of Object.entries(manifest())) {
    const built = (cases() as Record<string, Case>)[name];
    assert.equal(built?.constructor, classOf(message), name);
  }
});

test("each case encodes to the bytes rust writes", () => {
  for (const [name, value] of Object.entries(cases())) {
    assert.deepEqual(bytesOf(value), golden(name), name);
  }
});

test("each golden decodes to the value the case builds", () => {
  for (const [name, value] of Object.entries(cases())) {
    const decoded = classOf(manifest()[name] as string).decode(golden(name));
    assert.deepEqual(decoded, value, name);
  }
});

test("a decoded golden encodes back to the same bytes", () => {
  for (const name of Object.keys(cases())) {
    const data = golden(name);
    const decoded = classOf(manifest()[name] as string).decode(data);
    assert.deepEqual(bytesOf(decoded), data, name);
  }
});

test("a message with no fields is no bytes", () => {
  assert.equal(golden("ledger_show_request").length, 0);
  assert.deepEqual(api.LedgerShowRequest.decode(new Uint8Array(0)), new api.LedgerShowRequest());
});

test("a lease that is absent and one that is present are different bytes", () => {
  const absent = golden("outcome_ledger_show");
  const present = bytesOf(
    new api.Outcome({
      ledgerShow: response(
        new api.LeaseView({ holder: "ci-7", expiresMs: 4_600_000n, state: api.LeaseState.LAPSING, yours: true }),
      ),
    }),
  );
  assert.notDeepEqual(absent, present);
  assert.equal(api.Outcome.decode(absent).ledgerShow?.lease, undefined);
});
