// SPDX-License-Identifier: Apache-2.0
//! The mapping of coordinates onto mopm's grammars, and the checks of a set, all with no I/O.

mod common;

use std::collections::BTreeSet;

use common::{CHANNEL, Owned, Recording, candidates, config, coordinate, response};
use ucdk_release_mopm::{
    Key, MAX_FILE_BYTES, MAX_GROUP_BYTES, MAX_MEMBERS, SUBDIRS_V1, Subdir, Url, build_of_file,
    check_key, check_observe, check_reason, check_shape, members, read_version, safe_file_name,
    valid_build, valid_name, valid_reason, valid_sha256, valid_version,
};
use ucdk_release_proto::v1::{ArtifactType, Reason};
use ucdk_release_seams::SeamError;

const REFUSED: SeamError = SeamError::Refused(Reason::Channel);

fn is_permanent(e: &SeamError) -> bool {
    matches!(e, SeamError::Permanent(_))
}

#[test]
fn coordinates_that_differ_only_where_a_slash_falls_are_refused() {
    // Each pair joins to the same text with a '/'; none of them is a coordinate mopm can hold,
    // so none is mapped to anything, let alone to the same thing.
    let pairs = [
        (("pkg/1", "2.0.0"), ("pkg", "1/2.0.0")),
        (("a/b", "1.2.3"), ("a", "b/1.2.3")),
        (("a_b", "1.2/3.4"), ("a_b/1.2", "3.4")),
    ];
    for ((n1, v1), (n2, v2)) in pairs {
        for (name, version) in [(n1, v1), (n2, v2)] {
            let c = coordinate(name, version, "linux-x86_64");
            assert_eq!(check_key(CHANNEL, &c), Err(REFUSED), "{name}/{version}");
        }
    }
    let slash_platform = coordinate("demo", "1.2.3", "linux/x86_64");
    assert_eq!(check_key(CHANNEL, &slash_platform), Err(REFUSED));
}

#[test]
fn distinct_coordinates_have_distinct_keys() {
    let mut keys = BTreeSet::new();
    let mut count = 0;
    for name in ["ab", "a_b", "abb", "ab1", "a1b"] {
        for version in ["1.2.3", "1.2.30", "11.2.3", "1.22.3", "1.2.33"] {
            for subdir in Subdir::ALL {
                let c = coordinate(name, version, subdir.platform());
                let key = check_key(CHANNEL, &c).unwrap();
                assert_eq!(
                    key,
                    Key {
                        name: name.to_owned(),
                        version: version.to_owned(),
                        subdir
                    }
                );
                keys.insert(key);
                count += 1;
            }
        }
    }
    assert_eq!(keys.len(), count);
}

#[test]
fn a_slash_inside_a_segment_stays_inside_it() {
    let base = Url::parse("https://registry.example.com/api/v1/").unwrap();
    let joined = base.join(&["a/b"]);
    let split = base.join(&["a", "b"]);
    assert_eq!(joined.path(), "/api/v1/a%2Fb");
    assert_eq!(split.path(), "/api/v1/a/b");
    assert!(joined != split);
    assert_eq!(base.join(&["é ?#%"]).path(), "/api/v1/%C3%A9%20%3F%23%25");
    assert_eq!(base.join(&["a-b._~Z9"]).path(), "/api/v1/a-b._~Z9");
}

#[test]
fn the_platform_table_maps_both_ways() {
    let expected = [
        ("any", Subdir::Noarch, "noarch"),
        ("linux-x86_64", Subdir::Linux64, "linux-64"),
        ("linux-aarch64", Subdir::LinuxAarch64, "linux-aarch64"),
        ("darwin-aarch64", Subdir::OsxArm64, "osx-arm64"),
    ];
    for (platform, subdir, wire) in expected {
        assert_eq!(Subdir::of_platform(platform), Some(subdir));
        assert_eq!(subdir.platform(), platform);
        assert_eq!(subdir.as_str(), wire);
        assert_eq!(subdir.to_string(), wire);
        assert_eq!(Subdir::parse(wire), Some(subdir));
    }
    // A bijection: every subdirectory has one platform and every platform one subdirectory.
    let platforms: BTreeSet<_> = Subdir::ALL.iter().map(|s| s.platform()).collect();
    assert_eq!(platforms.len(), Subdir::ALL.len());
    assert_eq!(Subdir::ALL.len(), expected.len());
    for subdir in Subdir::ALL {
        assert_eq!(Subdir::of_platform(subdir.platform()), Some(subdir));
        assert_eq!(Subdir::parse(subdir.as_str()), Some(subdir));
    }
    for outside in [
        "darwin-arm64",
        "osx-arm64",
        "linux-64",
        "noarch",
        "Any",
        "",
        "windows-x86_64",
    ] {
        assert_eq!(Subdir::of_platform(outside), None, "{outside}");
    }
    for outside in ["linux-x86_64", "any", "OSX-ARM64", "", "win-64"] {
        assert_eq!(Subdir::parse(outside), None, "{outside}");
    }
}

#[test]
fn the_platform_table_right_hand_side_is_the_registry_subdirectory_list() {
    let wire: BTreeSet<&str> = Subdir::ALL.iter().map(|s| s.as_str()).collect();
    let registry: BTreeSet<&str> = SUBDIRS_V1.iter().copied().collect();
    assert_eq!(wire, registry);
    assert_eq!(
        SUBDIRS_V1,
        ["linux-64", "linux-aarch64", "noarch", "osx-arm64"]
    );
}

#[test]
fn a_coordinate_mopm_cannot_hold_is_refused() {
    let ok = coordinate("demo", "1.2.3", "linux-x86_64");
    assert!(check_key(CHANNEL, &ok).is_ok());

    let mut other_channel = ok.clone();
    other_channel.channel = "other".to_owned();
    let mut other_type = ok.clone();
    other_type.r#type = ArtifactType::File as i32;
    let mut unspecified = ok.clone();
    unspecified.r#type = 0;
    let bad_name = coordinate("Demo", "1.2.3", "linux-x86_64");
    let bad_version = coordinate("demo", "1.2.3-rc1", "linux-x86_64");
    let bad_platform = coordinate("demo", "1.2.3", "darwin-arm64");
    for refused in [
        other_channel,
        other_type,
        unspecified,
        bad_name,
        bad_version,
        bad_platform,
    ] {
        assert_eq!(check_key(CHANNEL, &refused), Err(REFUSED));
    }
}

#[test]
fn the_name_grammar_is_exact() {
    let long = "a".repeat(64);
    let too_long = "a".repeat(65);
    for name in ["ab", "a1", "a_", "demo_pkg2", long.as_str()] {
        assert!(valid_name(name), "{name}");
    }
    for name in [
        "",
        "a",
        "Ab",
        "aB",
        "1a",
        "_a",
        "my-pkg",
        "a b",
        "a.b",
        "é1",
        "ab\n",
        too_long.as_str(),
    ] {
        assert!(!valid_name(name), "{name:?}");
    }
}

#[test]
fn the_version_grammar_is_exact() {
    for version in ["0.0.0", "1.2.3", "10.20.30", "1.0.100"] {
        assert!(valid_version(version), "{version}");
    }
    for version in [
        "",
        "1",
        "1.0",
        "1.0.0.4567",
        "01.2.3",
        "1.02.3",
        "1.2.007",
        "1.0.0-rc1",
        "1.0.0+b",
        "v1.0.0",
        "1..3",
        ".1.2",
        "1.2.",
        "1.2.3 ",
        " 1.2.3",
        "1.2.3\n",
        "1.2.x",
        "1.2.-3",
        "٣.٢.١",
    ] {
        assert!(!valid_version(version), "{version:?}");
    }
}

#[test]
fn a_version_over_128_characters_is_refused() {
    let longest = format!("1.2.{}", "3".repeat(124));
    let over = format!("1.2.{}", "3".repeat(125));
    assert_eq!(longest.len(), 128);
    assert!(valid_version(&longest));
    assert!(!valid_version(&over));
    assert!(check_key(CHANNEL, &coordinate("demo", &longest, "any")).is_ok());
    assert_eq!(
        check_key(CHANNEL, &coordinate("demo", &over, "any")),
        Err(REFUSED)
    );
}

#[test]
fn an_invalid_version_never_produces_a_request() {
    let over = format!("1.2.{}", "3".repeat(125));
    let versions = [
        "1.0",
        "01.2.3",
        "1.0.0-rc1",
        "1.2.3.x",
        "",
        "v1.2.3",
        "1.2.3 ",
        "1.2.3\n",
        "١.٢.٣",
        over.as_str(),
    ];
    let config = config();
    let mut transport = Recording::default();
    for version in versions {
        let k = coordinate("demo", version, "linux-x86_64");
        let outcome = read_version(&mut transport, &config, &k);
        assert_eq!(outcome.unwrap_err(), REFUSED, "{version:?}");
    }
    for name in ["Demo", "my-pkg", "d", "demo/x"] {
        let k = coordinate(name, "1.2.3", "linux-x86_64");
        assert_eq!(
            read_version(&mut transport, &config, &k).unwrap_err(),
            REFUSED
        );
    }
    let k = coordinate("demo", "1.2.3", "darwin-arm64");
    assert_eq!(
        read_version(&mut transport, &config, &k).unwrap_err(),
        REFUSED
    );
    assert!(transport.sent.is_empty(), "{:?}", transport.sent);
}

#[test]
fn a_valid_coordinate_is_read_once_as_a_public_get() {
    let config = config();
    let mut transport = Recording::default();
    transport.replies.push_back(Ok(response(200, &[], "{}")));
    let k = coordinate("demo", "1.2.3", "linux-x86_64");
    let answer = read_version(&mut transport, &config, &k).unwrap();
    assert_eq!(answer.status, 200);
    assert_eq!(transport.sent.len(), 1);
    let sent = &transport.sent[0];
    assert!(sent.contains("GET"), "{sent}");
    assert!(
        sent.contains("https://registry.example.com/api/v1/packages/demo/1.2.3"),
        "{sent}"
    );
    assert!(
        !sent.contains("authorization"),
        "a read carries no token: {sent}"
    );
}

#[test]
fn a_read_that_cannot_reach_the_registry_is_transient() {
    let config = config();
    let mut transport = Recording::default();
    let k = coordinate("demo", "1.2.3", "any");
    let failed = read_version(&mut transport, &config, &k).unwrap_err();
    assert!(matches!(failed, SeamError::Transient(_)), "{failed:?}");
    assert_eq!(transport.sent.len(), 1);
}

#[test]
fn the_build_and_file_name_grammars_are_exact() {
    for build in ["mojo26_0", "mojo26.2_0", "mojo1.22.333_0"] {
        assert!(valid_build(build), "{build}");
    }
    for build in [
        "",
        "mojo_0",
        "mojo26.2_1",
        "mojo26.2",
        "Mojo26_0",
        "mojo26._0",
        "mojo.26_0",
        "mojo26..2_0",
        "mojo26.x_0",
        "26_0",
        "mojo26.2_00",
    ] {
        assert!(!valid_build(build), "{build:?}");
    }
    let good = "mojo-demo-1.2.3-mojo26.2_0.conda";
    assert_eq!(build_of_file(good, "demo", "1.2.3"), Some("mojo26.2_0"));
    let wrong = [
        "mojo-demo-1.2.4-mojo26.2_0.conda",
        "mojo-demo2-1.2.3-mojo26.2_0.conda",
        "conda-demo-1.2.3-mojo26.2_0.conda",
        "mojo-demo-1.2.3-mojo26.2_0.tar.bz2",
        "mojo-demo-1.2.3-mojo26.2_1.conda",
        "mojo-demo-1.2.3-.conda",
        "mojo-demo-1.2.3-mojo26.2_0.CONDA",
        "mojo-demo-1.2.3-mojo26.2_0.conda ",
        "other.conda",
    ];
    for file in wrong {
        assert_eq!(build_of_file(file, "demo", "1.2.3"), None, "{file}");
    }
}

#[test]
fn a_file_name_is_at_most_255_safe_characters() {
    let named = |parts: usize| format!("mojo-demo-1.2.3-mojo1{}_0.conda", ".1".repeat(parts));
    assert_eq!(named(113).len(), 255);
    assert!(build_of_file(&named(113), "demo", "1.2.3").is_some());
    assert_eq!(named(114).len(), 257);
    assert_eq!(build_of_file(&named(114), "demo", "1.2.3"), None);

    assert!(safe_file_name(&"a".repeat(255)));
    assert!(!safe_file_name(&"a".repeat(256)));
    for name in [
        "", ".hidden", "-x", "a b", "a/b", "a\\b", "é.conda", "a\n", "a:b",
    ] {
        assert!(!safe_file_name(name), "{name:?}");
    }
    for name in ["a", "A1._+-", "mojo-demo-1.2.3-mojo26.2_0.conda"] {
        assert!(safe_file_name(name), "{name:?}");
    }
}

#[test]
fn a_yank_reason_is_one_to_280_utf16_units() {
    assert!(valid_reason("x"));
    assert!(valid_reason(&"x".repeat(280)));
    assert!(!valid_reason(""));
    assert!(!valid_reason(&"x".repeat(281)));
    // One character outside the basic plane is two units for mopm.
    assert!(valid_reason(&"\u{1F600}".repeat(140)));
    assert!(!valid_reason(&"\u{1F600}".repeat(141)));
    assert!(valid_reason(&"\u{e9}".repeat(280)));
    assert_eq!(check_reason("withdrawn"), Ok(()));
    assert_eq!(check_reason(""), Err(REFUSED));
    assert_eq!(check_reason(&"x".repeat(281)), Err(REFUSED));
}

#[test]
fn a_digest_is_64_lower_case_hex_digits() {
    assert!(valid_sha256(&"0".repeat(64)));
    assert!(valid_sha256(&"abcdef0123456789".repeat(4)));
    for bad in [
        "".to_owned(),
        "0".repeat(63),
        "0".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
        format!("{} ", "0".repeat(63)),
    ] {
        assert!(!valid_sha256(&bad), "{bad:?}");
    }
}

#[test]
fn a_set_naming_a_coordinate_twice_is_permanent() {
    let a = coordinate("demo", "1.2.3", "linux-x86_64");
    let b = a.clone();
    let c = coordinate("demo", "1.2.3", "any");
    let refused = check_shape(CHANNEL, &[&a, &c, &b]).unwrap_err();
    assert!(is_permanent(&refused), "{refused:?}");
    let refused = check_observe(CHANNEL, &a, &[&a, &b]).unwrap_err();
    assert!(is_permanent(&refused), "{refused:?}");
    let owned = [
        Owned::new("demo", "1.2.3", "any"),
        Owned::new("demo", "1.2.3", "any"),
    ];
    assert!(is_permanent(
        &members(CHANNEL, &candidates(&owned)).unwrap_err()
    ));
}

#[test]
fn a_set_naming_two_versions_of_one_package_is_refused() {
    let a = coordinate("demo", "1.2.3", "any");
    let b = coordinate("demo", "1.2.4", "any");
    let other = coordinate("other", "1.2.3", "any");
    assert_eq!(
        check_shape(CHANNEL, &[&a, &other, &b]).unwrap_err(),
        REFUSED
    );
    assert_eq!(check_observe(CHANNEL, &a, &[&a, &b]).unwrap_err(), REFUSED);
    // The same version of two names, and of two platforms of one name, is fine.
    assert!(check_shape(CHANNEL, &[&a, &other]).is_ok());
}

#[test]
fn a_set_of_more_than_500_packages_is_refused() {
    let named = |n: usize| coordinate(&format!("p{n:03}"), "1.0.0", "any");
    let at_limit: Vec<_> = (0..MAX_MEMBERS).map(named).collect();
    let refs: Vec<_> = at_limit.iter().collect();
    assert_eq!(check_shape(CHANNEL, &refs).unwrap().len(), MAX_MEMBERS);
    let over = named(MAX_MEMBERS);
    let mut more = refs.clone();
    more.push(&over);
    assert_eq!(check_shape(CHANNEL, &more).unwrap_err(), REFUSED);
}

#[test]
fn observe_checks_its_coordinate_and_its_declared_set() {
    let k = coordinate("demo", "1.2.3", "linux-x86_64");
    let sibling = coordinate("demo", "1.2.3", "any");
    let key = check_observe(CHANNEL, &k, &[]).unwrap();
    assert_eq!(key.name, "demo");
    assert_eq!(key.subdir, Subdir::Linux64);
    assert_eq!(check_observe(CHANNEL, &k, &[&sibling, &k]).unwrap(), key);
    // A non-empty set lacking `k` is refused.
    assert_eq!(
        check_observe(CHANNEL, &k, &[&sibling]).unwrap_err(),
        REFUSED
    );
    // Grammar refusal holds for observe: the coordinate and every member of the set.
    let bad = coordinate("Demo", "1.2.3", "any");
    assert_eq!(check_observe(CHANNEL, &bad, &[]).unwrap_err(), REFUSED);
    assert_eq!(
        check_observe(CHANNEL, &k, &[&k, &bad]).unwrap_err(),
        REFUSED
    );
    let bad_version = coordinate("demo", "1.2", "linux-x86_64");
    assert_eq!(
        check_observe(CHANNEL, &bad_version, &[]).unwrap_err(),
        REFUSED
    );
}

#[test]
fn two_platforms_of_one_version_are_one_member_with_two_files() {
    let owned = [
        Owned::new("zeta", "2.0.0", "any"),
        Owned::new("demo", "1.2.3", "darwin-aarch64"),
        Owned::new("demo", "1.2.3", "linux-x86_64"),
    ];
    let found = members(CHANNEL, &candidates(&owned)).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].name, "demo");
    assert_eq!(found[1].name, "zeta");
    assert_eq!(found[0].version, "1.2.3");
    assert_eq!(found[0].build, "mojo26.2_0");
    assert_eq!(found[0].manifest_sha256, common::MANIFEST);
    let subdirs: Vec<_> = found[0].files.iter().map(|f| f.subdir).collect();
    assert_eq!(subdirs, [Subdir::Linux64, Subdir::OsxArm64]);
    assert_eq!(
        found[0].files[0].file_name,
        common::file_name("demo", "1.2.3")
    );
    assert_eq!(found[0].files[0].size, 1000);
    assert_eq!(found[0].files[0].sha256, common::DIGEST);
    assert_eq!(found[1].files.len(), 1);
}

#[test]
fn a_grammar_refusal_comes_before_any_file_check() {
    let mut bad = Owned::new("Demo", "1.2.3", "any");
    bad.size = 0;
    assert_eq!(members(CHANNEL, &candidates(&[bad])).unwrap_err(), REFUSED);
}

#[test]
fn an_empty_file_is_refused() {
    let mut owned = Owned::new("demo", "1.2.3", "any");
    owned.size = 0;
    let refused = members(CHANNEL, &candidates(&[owned])).unwrap_err();
    assert!(is_permanent(&refused), "{refused:?}");
}

#[test]
fn a_file_over_100_megabytes_is_refused_and_one_at_the_limit_is_not() {
    let mut owned = Owned::new("demo", "1.2.3", "any");
    owned.size = MAX_FILE_BYTES;
    assert!(members(CHANNEL, &candidates(std::slice::from_ref(&owned))).is_ok());
    owned.size = MAX_FILE_BYTES + 1;
    assert_eq!(
        members(CHANNEL, &candidates(&[owned])).unwrap_err(),
        REFUSED
    );
}

#[test]
fn a_group_over_20_gigabytes_is_refused_and_one_at_the_limit_is_not() {
    let make = |count: usize| -> Vec<Owned> {
        (0..count)
            .map(|n| {
                let mut o = Owned::new(&format!("p{n:03}"), "1.0.0", "any");
                o.size = MAX_FILE_BYTES;
                o
            })
            .collect()
    };
    let per_group = usize::try_from(MAX_GROUP_BYTES / MAX_FILE_BYTES).unwrap();
    let at_limit = make(per_group);
    assert!(members(CHANNEL, &candidates(&at_limit)).is_ok());
    let over = make(per_group + 1);
    assert_eq!(members(CHANNEL, &candidates(&over)).unwrap_err(), REFUSED);
}

#[test]
fn a_malformed_digest_is_refused() {
    let mut owned = Owned::new("demo", "1.2.3", "any");
    owned.sha256 = "ABC".to_owned();
    let refused = members(CHANNEL, &candidates(&[owned])).unwrap_err();
    assert!(is_permanent(&refused), "{refused:?}");
}

#[test]
fn a_file_name_that_is_not_the_coordinates_is_refused() {
    let wrong = [
        "mojo-demo-1.2.4-mojo26.2_0.conda",
        "mojo-other-1.2.3-mojo26.2_0.conda",
        "mojo-demo-1.2.3-mojo26.2_1.conda",
        "demo.conda",
    ];
    for file in wrong {
        let mut owned = Owned::new("demo", "1.2.3", "any");
        owned.file_name = file.to_owned();
        assert_eq!(
            members(CHANNEL, &candidates(&[owned])).unwrap_err(),
            REFUSED,
            "{file}"
        );
    }
}

#[test]
fn an_archive_that_disagrees_with_its_coordinate_is_refused() {
    type Edit = fn(&mut Owned);
    let edits: [(&str, Edit); 7] = [
        ("name", |o| o.archive.index_name = "mojo-other".to_owned()),
        ("version", |o| o.archive.index_version = "1.2.4".to_owned()),
        ("subdir", |o| {
            o.archive.index_subdir = "linux-aarch64".to_owned();
        }),
        ("build", |o| o.archive.index_build = "mojo26.3_0".to_owned()),
        ("manifest name", |o| {
            o.archive.manifest_name = "other".to_owned();
        }),
        ("manifest version", |o| {
            o.archive.manifest_version = Some("1.2.4".to_owned());
        }),
        ("no mojo prefix", |o| {
            o.archive.index_name = "demo".to_owned();
        }),
    ];
    for (what, edit) in edits {
        let mut owned = Owned::new("demo", "1.2.3", "any");
        edit(&mut owned);
        let refused = members(CHANNEL, &candidates(&[owned])).unwrap_err();
        assert!(is_permanent(&refused), "{what}: {refused:?}");
    }
    // A manifest without a version agrees with any.
    let mut owned = Owned::new("demo", "1.2.3", "any");
    owned.archive.manifest_version = None;
    assert!(members(CHANNEL, &candidates(&[owned])).is_ok());
}

#[test]
fn the_archive_subdirectory_must_be_the_one_the_table_maps_to() {
    for subdir in Subdir::ALL {
        let owned = Owned::new("demo", "1.2.3", subdir.platform());
        assert!(members(CHANNEL, &candidates(&[owned])).is_ok(), "{subdir}");
        for other in Subdir::ALL.into_iter().filter(|s| *s != subdir) {
            let mut owned = Owned::new("demo", "1.2.3", subdir.platform());
            owned.archive.index_subdir = other.as_str().to_owned();
            let refused = members(CHANNEL, &candidates(&[owned])).unwrap_err();
            assert!(is_permanent(&refused), "{subdir} as {other}: {refused:?}");
        }
    }
}

#[test]
fn the_archives_of_one_member_must_agree_on_one_manifest() {
    let pair = || {
        [
            Owned::new("demo", "1.2.3", "linux-x86_64"),
            Owned::new("demo", "1.2.3", "darwin-aarch64"),
        ]
    };
    assert!(members(CHANNEL, &candidates(&pair())).is_ok());

    let mut other_manifest = pair();
    other_manifest[1].archive.manifest_sha256 = common::DIGEST.to_owned();
    let mut other_library = pair();
    other_library[1].archive.native_library = Some("libdemo.dylib".to_owned());
    let mut only_one_library = pair();
    only_one_library[0].archive.native_library = Some("libdemo.so".to_owned());
    let mut other_build = pair();
    other_build[1].file_name = "mojo-demo-1.2.3-mojo26.3_0.conda".to_owned();
    other_build[1].archive.index_build = "mojo26.3_0".to_owned();
    for (what, owned) in [
        ("manifest", other_manifest),
        ("native library", other_library),
        ("native library on one side", only_one_library),
        ("build", other_build),
    ] {
        let refused = members(CHANNEL, &candidates(&owned)).unwrap_err();
        assert!(is_permanent(&refused), "{what}: {refused:?}");
    }

    // Archives of different packages need not agree with each other.
    let mut apart = [
        Owned::new("demo", "1.2.3", "any"),
        Owned::new("other", "9.9.9", "any"),
    ];
    apart[1].archive.manifest_sha256 = common::DIGEST.to_owned();
    assert!(members(CHANNEL, &candidates(&apart)).is_ok());
}
