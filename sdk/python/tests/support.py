# SPDX-License-Identifier: Apache-2.0
"""What the SDK tests share: the library, the scenario data of the command line, and the pair of
services every scenario starts from."""

import re
import sys
from pathlib import Path

from ucdk import Library, Session, api, proto, resources

REPO = Path(__file__).resolve().parents[3]
DATA = REPO / "crates/ucdk-cli/tests/data"
LIBRARY = REPO / "target/debug" / (
    "libucdk_capi.dylib" if sys.platform == "darwin" else "libucdk_capi.so"
)

IMAGE = proto.Image.digest("sha256:aa")


def library_path():
    """The library `cargo xtask sdk-test` builds."""
    if not LIBRARY.is_file():
        raise AssertionError("%s is not built; run `cargo xtask sdk-test`, which builds it" % LIBRARY)
    return LIBRARY


def library():
    return Library(library_path())


def open_session(provider="fake", **config):
    config.setdefault("ledger_kind", "memory")
    return Session(api.SessionConfig(provider=provider, **config), library_path=library_path())


def pair():
    """`example-svc` (public) calls `example-db`: the pair of the command line's scenario."""
    db = resources.Service("example-db", IMAGE, port=8080)
    svc = resources.Service(
        "example-svc", IMAGE, port=8080, exposure=resources.Service.Public(), uses=[db]
    )
    return resources.resource_list(svc, db)


# ---- the command line's recorded output ---------------------------------------------------------

ACTIONS = {
    "create": api.StepAction.CREATE,
    "update": api.StepAction.UPDATE,
    "ok": api.StepAction.NOOP,
    "delete": api.StepAction.DELETE,
    "wait": api.StepAction.CONVERGING,
    "later": api.StepAction.KNOWN_AFTER_APPLY,
    "reap": api.StepAction.REAP,
}


def read_plan(name):
    """(hash, [(action, path, kind)]) of a plan the command line printed."""
    lines = (DATA / name).read_text().splitlines()
    plan_hash = next(l[len("plan-hash: ") :] for l in lines if l.startswith("plan-hash: "))
    steps = []
    for line in lines:
        if line.startswith("  "):
            word, path, kind = line.split()
            steps.append((ACTIONS[word], path, kind.strip("[]")))
    return plan_hash, steps


def read_report(name):
    """{verb: [paths]} of a run the command line printed."""
    report = {}
    for line in (DATA / name).read_text().splitlines():
        if line.startswith("  "):
            verb, path = line.split()
            report.setdefault(verb, []).append(path)
    return report


RECORDS = {"none": None, "confirmed": api.RecordState.CONFIRMED}
LIVES = {"absent": None, "ready": api.LiveStatus.READY}
STANDINGS = {"agrees": api.Standing.AGREES, "not applied": api.Standing.NOT_APPLIED}


def read_status(name):
    """[(path, kind, record, live, standing)] of the status the command line printed."""
    out = []
    pattern = re.compile(r"  (\S+) \[(\S+)\] record: (\w+), live: (\w+), (.+)")
    for line in (DATA / name).read_text().splitlines():
        match = pattern.fullmatch(line)
        if match:
            path, kind, record, live, standing = match.groups()
            out.append((path, kind, RECORDS[record], LIVES[live], STANDINGS[standing]))
    return out


def steps_of(plan):
    return [(s.action, s.path, s.kind) for s in plan.steps]


def objects_of(outcome):
    return [
        (o.path, o.kind, o.record, o.live, o.standing) for o in outcome.status.objects
    ]
