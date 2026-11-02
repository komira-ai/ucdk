// SPDX-License-Identifier: Apache-2.0
//! The id of a group: the vectors mopm documents for its derivation, and what each part of the
//! declared set does to it.

use std::collections::BTreeSet;

use ucdk_release_mopm::{Member, MemberFile, Subdir, group_id, valid_group_id};

fn file(subdir: Subdir, version: &str, digit: char, size: u64) -> MemberFile {
    MemberFile {
        subdir,
        file_name: format!("mojo-alpha-{version}-mojo1.0.0_0.conda"),
        size,
        sha256: digit.to_string().repeat(64),
    }
}

fn member(name: &str, version: &str, files: Vec<MemberFile>) -> Member {
    Member {
        name: name.to_owned(),
        version: version.to_owned(),
        build: "mojo1.0.0_0".to_owned(),
        manifest_sha256: "0".repeat(64),
        files,
    }
}

fn alpha(digit: char) -> Vec<Member> {
    vec![member(
        "alpha",
        "1.0.0",
        vec![file(Subdir::Linux64, "1.0.0", digit, 1000)],
    )]
}

/// The set of mopm's third vector: members and files in the order its request sends them.
fn two_members() -> Vec<Member> {
    let beta = MemberFile {
        subdir: Subdir::Noarch,
        file_name: "mojo-beta-0.3.0-mojo1.0.0_0.conda".to_owned(),
        size: 512,
        sha256: "d".repeat(64),
    };
    vec![
        member("beta", "0.3.0", vec![beta]),
        member(
            "alpha",
            "2.1.0",
            vec![
                file(Subdir::OsxArm64, "2.1.0", 'c', 4096),
                file(Subdir::Linux64, "2.1.0", 'b', 2048),
            ],
        ),
    ]
}

#[test]
fn the_documented_vectors_give_the_documented_ids() {
    assert_eq!(
        group_id("u:1234", &alpha('a')),
        "grp_750db8c095f4c2517cedf12a308149e5"
    );
    assert_eq!(
        group_id("u:4321", &alpha('a')),
        "grp_57794d9daef2aedabe009f48791f7f3a"
    );
    assert_eq!(
        group_id("ci:gh-repo:987654", &two_members()),
        "grp_890c960efd66fe95fc3289b62641f192"
    );
    assert_eq!(
        group_id("u:1234", &alpha('b')),
        "grp_d0c9541cb93d9763a8f4f9d73fc4fb93"
    );
}

#[test]
fn the_order_of_members_and_files_is_not_part_of_the_id() {
    let mut reversed = two_members();
    reversed.reverse();
    for m in &mut reversed {
        m.files.reverse();
    }
    assert_eq!(
        group_id("ci:gh-repo:987654", &reversed),
        group_id("ci:gh-repo:987654", &two_members())
    );
}

#[test]
fn the_actor_the_version_and_each_field_of_a_file_are_part_of_the_id() {
    let base = alpha('a');
    let edit = |change: fn(&mut Member)| {
        let mut set = alpha('a');
        change(&mut set[0]);
        group_id("u:1234", &set)
    };
    let ids: BTreeSet<String> = [
        group_id("u:1234", &base),
        group_id("u:1235", &base),
        edit(|m| m.version = "1.0.1".to_owned()),
        edit(|m| m.name = "alpha2".to_owned()),
        edit(|m| m.files[0].subdir = Subdir::Noarch),
        edit(|m| m.files[0].size = 1001),
        edit(|m| m.files[0].sha256 = "f".repeat(64)),
        edit(|m| m.files[0].file_name = "mojo-alpha-1.0.0-mojo2.0.0_0.conda".to_owned()),
    ]
    .into_iter()
    .collect();
    assert_eq!(ids.len(), 8);
}

#[test]
fn a_group_id_is_grp_and_32_lower_case_hex_digits() {
    assert!(valid_group_id("grp_750db8c095f4c2517cedf12a308149e5"));
    for bad in [
        "",
        "grp_",
        "750db8c095f4c2517cedf12a308149e5",
        "grp_750db8c095f4c2517cedf12a308149e",
        "grp_750db8c095f4c2517cedf12a308149e55",
        "grp_750DB8C095F4C2517CEDF12A308149E5",
        "grp_750db8c095f4c2517cedf12a308149eg",
        "xrp_750db8c095f4c2517cedf12a308149e5",
    ] {
        assert!(!valid_group_id(bad), "{bad}");
    }
}
