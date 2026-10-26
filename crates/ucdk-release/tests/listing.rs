// SPDX-License-Identifier: Apache-2.0
//! Reading the output listing of Buck2 and Bazel: one output per target, from what the build
//! system printed.

use ucdk_release::listing::{ListingError, Output, bazel, buck2};
use ucdk_release_proto::v1::Reason;

const LABEL: &str = "//release:tool";

fn output(label: &str, path: &str) -> Output {
    Output {
        label: label.to_owned(),
        path: path.to_owned(),
    }
}

#[test]
fn a_buck2_listing_line_is_split_at_the_first_space() {
    assert_eq!(
        buck2(LABEL, b"//release:tool /root/buck-out/v2/tool\n"),
        Ok(output(LABEL, "/root/buck-out/v2/tool"))
    );
    // The path may hold spaces: only the first one separates the label.
    assert_eq!(
        buck2(
            LABEL,
            b"root//release:tool /root/my project/out dir/tool.bin\n"
        ),
        Ok(output(
            "root//release:tool",
            "/root/my project/out dir/tool.bin"
        ))
    );
    // Blank lines and a missing final newline do not matter, and a carriage return is not part
    // of the path.
    assert_eq!(
        buck2(LABEL, b"\n//release:tool /out/tool\r\n\n"),
        Ok(output(LABEL, "/out/tool"))
    );
    assert_eq!(
        buck2(LABEL, b"//release:tool /out/tool"),
        Ok(output(LABEL, "/out/tool"))
    );
}

#[test]
fn zero_or_two_outputs_are_build_output() {
    for listing in [&b""[..], b"\n  \n"] {
        let err = buck2(LABEL, listing).expect_err("no output");
        assert_eq!(
            err,
            ListingError::None {
                label: LABEL.to_owned()
            }
        );
        assert_eq!(err.reason(), Reason::BuildOutput);
        assert!(err.to_string().contains("lists no output"), "{err}");
    }
    let err = buck2(LABEL, b"//release:tool /out/a\n//release:tool /out/b\n").expect_err("two");
    assert_eq!(
        err,
        ListingError::Several {
            label: LABEL.to_owned(),
            found: vec!["/out/a".to_owned(), "/out/b".to_owned()]
        }
    );
    assert_eq!(err.reason(), Reason::BuildOutput);
    // The fault names the label and every output found.
    let text = err.to_string();
    assert!(text.contains(LABEL) && text.contains("2 outputs") && text.contains("/out/a, /out/b"));

    // The same for Bazel: no output, and two.
    let none = bazel(LABEL, b"", b"/exec\n").expect_err("none");
    assert_eq!(none.reason(), Reason::BuildOutput);
    assert!(matches!(none, ListingError::None { .. }));
    let two = bazel(LABEL, b"bazel-out/a\nbazel-out/b\n", b"/exec\n").expect_err("two");
    assert_eq!(
        two,
        ListingError::Several {
            label: LABEL.to_owned(),
            found: vec![
                "/exec/bazel-out/a".to_owned(),
                "/exec/bazel-out/b".to_owned()
            ]
        }
    );
}

#[test]
fn a_bazel_output_is_the_execution_root_joined_with_its_relative_path() {
    assert_eq!(
        bazel(
            LABEL,
            b"bazel-out/k8-fastbuild/bin/release/tool\n",
            b"/work/_bazel/exec\n"
        ),
        Ok(output(
            LABEL,
            "/work/_bazel/exec/bazel-out/k8-fastbuild/bin/release/tool"
        ))
    );
    // A trailing slash of the root does not double.
    assert_eq!(
        bazel(LABEL, b"out/tool\n", b"/exec/\n"),
        Ok(output(LABEL, "/exec/out/tool"))
    );
}

#[test]
fn a_buck2_line_must_name_the_target_asked_for() {
    // The label itself, and a cell-qualified form of a label that starts with `//`.
    assert_eq!(
        buck2("root//a:b", b"root//a:b /out/b\n"),
        Ok(output("root//a:b", "/out/b"))
    );
    assert_eq!(
        buck2("//a:b", b"root//a:b /out/b\n"),
        Ok(output("root//a:b", "/out/b"))
    );
    // Another target is a fault, with the path it printed not taken.
    for (label, listing) in [
        ("root//a:b", &b"other//x:y /etc/shadow\n"[..]),
        ("root//a:b", b"other//a:b /out/b\n"),
        ("//a:b", b"root//a:c /out/b\n"),
        ("//a:b", b"root//x/a:b /out/b\n"),
        ("//a:b", b"a:b /out/b\n"),
        ("//a:b", b"//a:b2 /out/b\n"),
    ] {
        let err = buck2(label, listing).expect_err("another target");
        assert!(
            matches!(&err, ListingError::Malformed { label: l, why }
                if l == label && why.contains("not the target asked for")),
            "{label}: {err:?}"
        );
        assert_eq!(err.reason(), Reason::BuildOutput);
    }
}

#[test]
fn a_path_with_a_control_character_is_malformed() {
    for path in [
        "/out/to\0ol",
        "/out/x\ty",
        "/out/x\u{1b}[0m",
        "/out/a\rb",
        "/out/x\u{85}y",
    ] {
        let line = format!("//release:tool {path}\n");
        let err = buck2(LABEL, line.as_bytes()).expect_err("control character");
        assert!(
            matches!(&err, ListingError::Malformed { .. }),
            "{path:?}: {err:?}"
        );
        let rel = format!("{}\n", &path[1..]);
        let err = bazel(LABEL, rel.as_bytes(), b"/exec\n").expect_err("control in output");
        assert!(
            matches!(&err, ListingError::Malformed { .. }),
            "{path:?}: {err:?}"
        );
    }
    let err = bazel(LABEL, b"out/tool\n", b"/ex\tec\n").expect_err("control in root");
    assert!(matches!(&err, ListingError::Malformed { .. }), "{err:?}");
}

#[test]
fn a_listing_that_cannot_be_read_is_build_output() {
    let malformed = |err: ListingError| {
        assert!(
            matches!(&err, ListingError::Malformed { label, .. } if label == LABEL),
            "{err:?}"
        );
        assert_eq!(err.reason(), Reason::BuildOutput);
        assert!(err.to_string().contains("cannot be read"), "{err}");
    };
    // Buck2: no space, a path that is not absolute, a path that climbs, bytes that are not text.
    malformed(buck2(LABEL, b"//release:tool\n").expect_err("no space"));
    malformed(buck2(LABEL, b"//release:tool out/tool\n").expect_err("relative"));
    malformed(buck2(LABEL, b"//release:tool /out/../etc/tool\n").expect_err("climbs"));
    malformed(buck2(LABEL, b"//release:tool /out/\xff\n").expect_err("not UTF-8"));
    // Bazel: an output that is absolute or climbs, and a root that is not one absolute path.
    malformed(bazel(LABEL, b"/etc/tool\n", b"/exec\n").expect_err("absolute output"));
    malformed(bazel(LABEL, b"a/../../tool\n", b"/exec\n").expect_err("climbing output"));
    for root in [&b""[..], b"exec\n", b"/a\n/b\n", b"/\n", b"/exec/../x\n"] {
        malformed(bazel(LABEL, b"out/tool\n", root).expect_err("bad root"));
    }
    malformed(bazel(LABEL, b"out/tool\n", b"/\xff\n").expect_err("root not UTF-8"));
    malformed(bazel(LABEL, b"out/\xff\n", b"/exec\n").expect_err("output not UTF-8"));
}
