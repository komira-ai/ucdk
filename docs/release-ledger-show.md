# ucdk release: `ledger show`

Status: Decided here. This page is the full text of section 3.4 of [release.md](release.md), split out to keep that file under the project's size bound. Section numbers without a file name are those of release.md.

## 3.4 `ledger show`: the messages and what each golden sets

`ucdk_ledger_show` and `ucdk ledger show` are the one verb the deletion PR adds. They show one ledger document as data. Messages, in `ucdk.api.v1` (the numbers are final; ordered lists, never maps, so every host prints the same bytes):

```proto
message LedgerShowRequest {}                    // the ledger is the session's (SessionConfig.ledger_kind, ledger_location)
enum LeaseState { LEASE_STATE_UNSPECIFIED = 0; LEASE_STATE_LIVE = 1; LEASE_STATE_LAPSING = 2; LEASE_STATE_LAPSED = 3; }
enum RecordOrigin { RECORD_ORIGIN_UNSPECIFIED = 0; RECORD_ORIGIN_CREATED = 1; RECORD_ORIGIN_ADOPTED = 2; }
message Pair { string key = 1; string value = 2; }
message LeaseView {
  string holder = 1;
  uint64 expires_ms = 2;                         // milliseconds since the Unix epoch, by the ledger's clock
  LeaseState state = 3;                          // by the session's clock; LAPSING = expired but still inside MAX_CLOCK_SKEW, so others are refused
  bool yours = 4;                                // holder == SessionConfig.holder
}
message LedgerRecord {
  string path = 1; string kind = 2; string name = 3;
  RecordOrigin origin = 4; RecordState state = 5;
  string physical = 6; repeated string deps = 7;
  repeated Pair outputs = 8;                     // key order
}
message LedgerShowResponse {
  string token = 1;                              // the document's token, text form
  uint64 fence = 2;                              // the highest fence ever granted
  LeaseView lease = 3;                           // absent (message presence): no lease is recorded
  repeated LedgerRecord records = 4;             // path order
}
// Outcome gains: LedgerShowResponse ledger_show = 9;  Error error = 10;   (3.1, ucdk-api row)
```

- **What it promises.** A call takes no lease and changes no file and no byte of the ledger: a file ledger is opened, on each call, with `FileLedger::open_read_only_with_clock` (3.1), which creates no lock file, takes no lock and removes no temporary. It contends with no lock holder and works in a read-only directory. The claim is tested as exactly that, not as "writes nothing" in general.
- **What it reports.** `fence`, `lease` and the records come from `Snapshot` (`fence`, `lease`, `at`, `records`, 3.1); `lease.state` is `Holding::state(at)`, where `at` is the reading of the clock the read-only ledger was opened with, which is the session's clock (`test_clock_ms` when set, else the wall clock; `file::a_read_only_ledger_reports_the_clock_it_was_given` holds that a file ledger obeys it), and `yours` compares `holder`. This is what makes `SessionConfig.holder` and `test_clock_ms` observable in the deletion PR with no verb that takes a lease.
- **Failures.** Bytes that do not decode as a `LedgerShowRequest` are status `DECODE` of the call, as for every verb (3.1, `ucdk-capi` row), and are not an `Outcome`; the message has no field, so any bytes that decode are a valid request, and `Outcome.invalid` has no producer in the deletion PR (3.1, `ucdk-api` row). A ledger that cannot be opened or read is `Outcome.error` with `LEDGER` and exit class FAILED_NO_EFFECT (`ledger show` takes no lock, so it never answers `LEDGER_BUSY`; the verbs that open a file ledger with its lock do, from PR 10); a newer-major file is named in the message. This is possible because a file ledger is opened per call and not at session open (3.1, `ucdk-capi` row).
- **The goldens that set each surviving field** (`golden_coverage` fails by name on a field no golden sets, so each row is a file in `sdk/wire-golden` listed in `manifest.txt`):

| Golden | Type | Fields it sets |
|---|---|---|
| `session_config.bin` | `SessionConfig` | `ledger_kind`, `ledger_location`, `holder`, `test_clock_ms` |
| `error.bin` | `Error` | `code`, `message` |
| `outcome_invalid.bin` | `Outcome` | `invalid` (with `Error.code`, `Error.message`), `exit` = USAGE |
| `ledger_show_request.bin` | `LedgerShowRequest` | none (the message has no field; the file is empty and is read back as the message) |
| `ledger_show_response.bin` | `LedgerShowResponse` | `token`, `fence`, `lease` (`holder`, `expires_ms`, `state`, `yours`), `records` with every `LedgerRecord` field set (`path`, `kind`, `name`, `origin` = ADOPTED, `state`, `physical`, two `deps`, two `outputs`) |
| `outcome_ledger_show.bin` | `Outcome` | `ledger_show` over a response whose `lease` is absent (so presence is exercised both ways) |
| `outcome_error.bin` | `Outcome` | `error` (code `LEDGER`), `exit` = FAILED_NO_EFFECT |

`Outcome.exit` is set to a non-default value by two goldens; a zero `exit` (DONE) is the proto3 default and no golden could set it. The floor in `golden_coverage` is an equality with the field count (3.1). Tests over the verb are named in PR 2 (11).
