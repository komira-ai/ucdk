# SPDX-License-Identifier: Apache-2.0
"""The typed resource templates: what they ask for, and the wire messages they make."""

import unittest

from ucdk.generated import google_protobuf as gp
from ucdk.generated import resources as r
from ucdk.generated import ucdk_resource_v1 as pb

IMAGE = pb.Image.digest("sha256:aa")


class Templates(unittest.TestCase):
    def test_a_service_needs_an_id_and_an_image(self):
        with self.assertRaises(TypeError):
            r.Service("web")
        with self.assertRaises(TypeError):
            r.Service(image=IMAGE)
        with self.assertRaises(TypeError):
            r.Job("nightly")

    def test_a_misspelt_field_is_a_python_error(self):
        with self.assertRaises(TypeError):
            r.Service("web", IMAGE, prt=80)

    def test_the_optional_fields_are_keywords_only(self):
        with self.assertRaises(TypeError):
            r.Service("web", IMAGE, 8080)

    def test_the_resource_it_makes(self):
        service = r.Service(
            "web",
            IMAGE,
            port=8080,
            args=["--x"],
            size=pb.Size(cpu_millis=500, memory_mb=256),
            scale=pb.Scale(min=0, max=3),
            health_path="/up",
            request_timeout=gp.Duration(seconds=5),
            max_concurrency=10,
            secret_env={"S": pb.SecretRef(name="sec")},
            exposure=r.Service.Public(),
        )
        self.assertEqual(
            service.to_resource(),
            pb.Resource(
                id="web",
                service=pb.Service(
                    image=IMAGE,
                    port=8080,
                    args=["--x"],
                    size=pb.Size(cpu_millis=500, memory_mb=256),
                    scale=pb.Scale(min=0, max=3),
                    health_path="/up",
                    request_timeout=gp.Duration(seconds=5),
                    max_concurrency=10,
                    secret_env={"S": pb.SecretRef(name="sec")},
                    public=pb.Service_Public(),
                ),
            ),
        )

    def test_a_job_and_its_trigger(self):
        job = r.Job("nightly", IMAGE, max_retries=0, trigger=r.Job.Schedule(cron="0 * * * *"))
        self.assertEqual(
            job.to_resource(),
            pb.Resource(
                id="nightly",
                job=pb.Job(
                    image=IMAGE, max_retries=0, schedule=pb.Job_Schedule(cron="0 * * * *")
                ),
            ),
        )
        on_demand = r.Job("run", IMAGE, trigger=r.Job.OnDemand())
        self.assertEqual(on_demand.body.which_trigger(), "on_demand")
        self.assertIsNone(r.Job("plain", IMAGE).body.which_trigger())

    def test_a_oneof_takes_only_its_own_arms(self):
        with self.assertRaises(TypeError):
            r.Service("web", IMAGE, exposure=r.Job.OnDemand())
        with self.assertRaises(TypeError):
            r.Service("web", IMAGE, exposure="public")

    def test_what_a_kind_is(self):
        self.assertEqual(r.Service.KIND, "service")
        self.assertEqual(r.Job.KIND, "job")
        self.assertEqual(r.Service.PORTABILITY, pb.Portability.PORTABLE)
        self.assertEqual(r.Job.PORTABILITY, pb.Portability.PORTABLE)
        self.assertEqual(r.Service.EXPOSES, (pb.Output.URL, pb.Output.HOST))
        self.assertEqual(r.Job.EXPOSES, ())

    def test_an_output_is_a_typed_reference(self):
        web = r.Service("web", IMAGE)
        self.assertEqual(web.url, pb.Ref(resource="web", standard=pb.Output.URL))
        self.assertEqual(web.host, pb.Ref(resource="web", standard=pb.Output.HOST))
        self.assertEqual(web.ref, pb.Ref(resource="web"))
        self.assertFalse(hasattr(r.Job("nightly", IMAGE), "url"))

    def test_an_environment_value_may_be_a_literal_a_reference_or_a_value(self):
        db = r.Service("db", IMAGE)
        web = r.Service(
            "web", IMAGE, env={"A": "x", "B": db.url, "C": pb.Value.param("p")}
        )
        self.assertEqual(
            web.body.env,
            {
                "A": pb.Value(literal="x"),
                "B": pb.Value(ref=pb.Ref(resource="db", standard=pb.Output.URL)),
                "C": pb.Value(param="p"),
            },
        )
        self.assertEqual(r.Service("plain", IMAGE).body.env, {})

    def test_uses_may_be_a_template_a_reference_or_a_uses(self):
        db = r.Service("db", IMAGE)
        explicit = pb.Uses(target=pb.Ref(resource="cache"), access=pb.Access.CALL)
        web = r.Service("web", IMAGE, uses=[db, pb.Ref(resource="queue"), explicit])
        self.assertEqual(
            web.to_resource().uses,
            [
                pb.Uses(target=pb.Ref(resource="db"), access=pb.Access.CALL),
                pb.Uses(target=pb.Ref(resource="queue"), access=pb.Access.CALL),
                explicit,
            ],
        )

    def test_a_resource_list_takes_templates_and_resources(self):
        built = pb.Resource(id="x", job=pb.Job(image=IMAGE))
        listed = r.resource_list(r.Service("web", IMAGE), built)
        self.assertEqual([x.id for x in listed.resource], ["web", "x"])
        self.assertIs(listed.resource[1], built)
        self.assertEqual(pb.ResourceList.decode(listed.encode()), listed)

    def test_the_template_docstring_says_what_the_kind_is(self):
        self.assertIn("The `service` resource.", r.Service.__doc__)
        self.assertIn("A container that answers requests", r.Service.__doc__)


if __name__ == "__main__":
    unittest.main()
