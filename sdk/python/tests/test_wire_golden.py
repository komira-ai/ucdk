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

from ucdk.generated import google_protobuf as gp
from ucdk.generated import ucdk_api_v1 as api
from ucdk.generated import ucdk_resource_v1 as pb

GOLDEN = Path(__file__).resolve().parents[2] / "wire-golden"


def step_image():
    return pb.Image.output("build", "img", platform="linux/amd64")


def digest_image():
    return pb.Image.digest("sha256:aa", platform="linux/amd64")


def secret(name):
    return pb.SecretRef(name=name, store="vault", version="7")


def full_service(**exposure_and_scale):
    return pb.Service(
        image=step_image(),
        port=8080,
        args=["--a", "--b"],
        env={
            "K": pb.Value.literal("v"),
            "P": pb.Value.param("p"),
            "R": pb.Value.ref(pb.Ref.standard(pb.Output.URL, resource="db")),
            "N": pb.Value.ref(pb.Ref.named("cert", resource="db")),
        },
        size=pb.Size(cpu_millis=500, memory_mb=256),
        health_path="/healthz",
        request_timeout=gp.Duration(seconds=30, nanos=5),
        max_concurrency=80,
        secret_env={"S": secret("sec")},
        **exposure_and_scale,
    )


def full_job(max_retries, **trigger):
    return pb.Job(
        image=digest_image(),
        args=["go"],
        size=pb.Size(cpu_millis=1000, memory_mb=512),
        max_retries=max_retries,
        timeout=gp.Duration(seconds=-600, nanos=-1),
        env={"E": pb.Value.literal("e")},
        secret_env={"T": secret("tok")},
        **trigger,
    )


def resource_list():
    return pb.ResourceList(
        resource=[
            pb.Resource(
                id="web",
                uses=[pb.Uses(target=pb.Ref(resource="db"), access=pb.Access.CALL)],
                service=full_service(
                    public=pb.Service_Public(), scale=pb.Scale(min=0, max=10)
                ),
            ),
            pb.Resource(
                id="nightly",
                job=full_job(3, schedule=pb.Job_Schedule(cron="0 * * * *", timezone="UTC")),
            ),
        ]
    )


def retained():
    return api.Retained(reason=api.RetainReason.UNDELETABLE, detail="held by the provider")


def report():
    return api.Report(
        created=["a/run", "b/run"],
        updated=["c/run"],
        resumed=["d/run"],
        deleted=["e/run"],
        reaped=["f/run"],
        retained=[api.RetainedObject(path="g/run", retained=retained())],
        converging=["h/run"],
        failed=["i/run"],
        blocked_by_failed=[api.Blocked(path="j/run", producer="i/run")],
        rolled_back=["k/run"],
        rollback_failed=[api.RollbackFailure(path="l/run", message="gone")],
    )


def error():
    return api.Error(code=api.ErrorCode.PROVIDER, message="the provider said no")


def finding():
    return api.Finding(
        resource="web",
        field="service.env.DB_URL",
        kind=api.FindingKind.UNBOUND_REF,
        message="db is not in the list",
    )


def refused(**reason):
    return api.Outcome(
        refused=api.Refusal(message="refused", **reason), exit=api.ExitClass.REFUSED
    )


def cases():
    plan = api.Plan(
        steps=[
            api.Step(path="web/run", kind="service", action=api.StepAction.CREATE),
            api.Step(
                path="old/run",
                kind="service",
                action=api.StepAction.RETAIN,
                physical_id="fake-1",
                retained=retained(),
            ),
        ],
        hash="ab" * 32,
    )
    return {
        "resource_list": resource_list(),
        "service_internal": full_service(
            internal=pb.Service_Internal(), scale=pb.Scale(max=10)
        ),
        "job_on_demand": full_job(None, on_demand=pb.Job_OnDemand()),
        "map_defaults": pb.Service(
            env={"": pb.Value()}, secret_env={"": pb.SecretRef()}
        ),
        "session_config": api.SessionConfig(
            provider="fake",
            provider_options={"region": "r1", "a": "b"},
            ledger_kind="file",
            ledger_location="/var/ucdk/ledger",
            cell="machine/cell/uid",
            credentials=api.CredentialSource.PROFILE,
            profile="work",
        ),
        "plan_request": api.PlanRequest(resources=resource_list(), mode=api.Mode.DESTROY),
        "apply_request": api.ApplyRequest(
            resources=resource_list(), plan_hash="cd" * 32, rollback=True
        ),
        "destroy_request": api.DestroyRequest(
            resources=resource_list(), plan_hash="cd" * 32
        ),
        "status_request": api.StatusRequest(resources=resource_list()),
        "outcome_planned": api.Outcome(planned=plan, exit=api.ExitClass.DONE),
        "outcome_done": api.Outcome(done=report(), exit=api.ExitClass.DONE),
        "outcome_refused_findings": refused(findings=api.Findings(findings=[finding()])),
        "outcome_refused_conflicts": refused(
            conflicts=api.Conflicts(
                conflicts=[
                    api.Conflict(
                        path="web/run",
                        kind=api.ConflictKind.OTHER_TOOL,
                        marker="other-tool",
                        message="owned by another tool",
                    )
                ]
            )
        ),
        "outcome_refused_empty_scope": refused(empty_scope=api.EmptyScope()),
        "outcome_refused_plan_hash_mismatch": refused(
            plan_hash_mismatch=api.PlanHashMismatch(approved="aa" * 32, plan="bb" * 32)
        ),
        "outcome_refused_plan_changed": refused(
            plan_changed=api.PlanChanged(approved="aa" * 32, now="bb" * 32)
        ),
        "outcome_refused_unsupported_stamp_mode": refused(
            unsupported_stamp_mode=api.UnsupportedStampMode(paths=["web/run"])
        ),
        "outcome_refused_unhosted_kinds": refused(
            unhosted_kinds=api.UnhostedKinds(
                kinds=[api.UnhostedKind(path="batch/run", kind="job")]
            )
        ),
        "outcome_failed": api.Outcome(
            failed=api.Failure(report=report(), at="web/run", error=error()),
            exit=api.ExitClass.FAILED_CHANGED,
        ),
        "outcome_invalid": api.Outcome(invalid=error(), exit=api.ExitClass.USAGE),
        "outcome_status": api.Outcome(
            status=api.StatusResponse(
                objects=[
                    api.ObjectStatus(
                        path="web/run",
                        kind="service",
                        record=api.RecordState.CONFIRMED,
                        live=api.LiveStatus.READY,
                        standing=api.Standing.AGREES,
                        outputs={"URL": "https://web.example.com"},
                    )
                ]
            ),
            exit=api.ExitClass.DONE,
        ),
        "catalog_response": api.CatalogResponse(
            nodes=[
                api.CatalogNode(
                    path="web/run", kind="service", wanted=True, deps=["db/run"]
                )
            ],
            findings=[finding()],
            exit=api.ExitClass.REFUSED,
        ),
        "error": error(),
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
    module = {"ucdk.api.v1": api, "ucdk.resource.v1": pb}[package]
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


if __name__ == "__main__":
    unittest.main()
