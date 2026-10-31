// SPDX-License-Identifier: Apache-2.0
//! The readers of mopm's success bodies: what a well-formed answer yields and that a body of any
//! other shape is refused whole.

mod common;

use common::{alpha, created_json, group_json, put_url, record_json, set};
use serde_json::{Value, json};
use ucdk_release_mopm::{
    GroupStatus, Subdir, VersionState, parse_created, parse_group, parse_record,
};

const ID: &str = "grp_750db8c095f4c2517cedf12a308149e5";
const OTHER: &str = "grp_57794d9daef2aedabe009f48791f7f3a";

fn bytes(value: &Value) -> Vec<u8> {
    value.to_string().into_bytes()
}

#[test]
fn every_group_status_mopm_lists_is_read_and_any_other_is_not() {
    let all = [
        ("open", GroupStatus::Open),
        ("validating", GroupStatus::Validating),
        ("committed", GroupStatus::Committed),
        ("visible", GroupStatus::Visible),
        ("rejected", GroupStatus::Rejected),
        ("failed", GroupStatus::Failed),
        ("expired", GroupStatus::Expired),
        ("aborted", GroupStatus::Aborted),
        ("superseded", GroupStatus::Superseded),
    ];
    for (text, status) in all {
        assert_eq!(GroupStatus::parse(text), Some(status), "{text}");
    }
    assert_eq!(GroupStatus::parse("finished"), None);
    assert_eq!(GroupStatus::parse("Open"), None);
}

#[test]
fn a_group_read_gives_the_declared_files_and_whether_each_is_stored() {
    let members = set(&alpha());
    let body = bytes(&group_json(ID, "open", &members, &["linux-64"]));
    let group = parse_group(&body).unwrap();
    assert_eq!(group.id, ID);
    assert_eq!(group.status, GroupStatus::Open);
    assert_eq!(group.members.len(), 1);
    let files = &group.members[0].files;
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].subdir, Subdir::Linux64);
    assert_eq!(files[0].uploaded, Some(true));
    assert_eq!(files[1].subdir, Subdir::Noarch);
    assert_eq!(files[1].uploaded, Some(false));
    // A group that is not open says nothing of storage.
    let body = bytes(&group_json(ID, "committed", &members, &[]));
    let group = parse_group(&body).unwrap();
    assert_eq!(group.status, GroupStatus::Committed);
    assert_eq!(group.members[0].files[0].uploaded, None);
}

#[test]
fn a_group_body_with_a_wrong_or_missing_field_is_not_read() {
    let members = set(&alpha());
    let good = group_json(ID, "open", &members, &[]);
    assert!(parse_group(&bytes(&good)).is_some());
    assert_eq!(parse_group(b"not json"), None);
    let broken = |change: &dyn Fn(&mut Value)| {
        let mut value = good.clone();
        change(&mut value);
        parse_group(&bytes(&value))
    };
    assert_eq!(broken(&|v| v["group_id"] = json!("grp_short")), None);
    assert_eq!(broken(&|v| v["group_id"] = json!(7)), None);
    assert_eq!(broken(&|v| v["status"] = json!("finished")), None);
    assert_eq!(broken(&|v| v["status"] = json!(false)), None);
    assert_eq!(broken(&|v| v["members"] = json!("none")), None);
    assert_eq!(broken(&|v| v["members"][0]["name"] = json!(3)), None);
    assert_eq!(broken(&|v| v["members"][0]["version"] = json!(3)), None);
    assert_eq!(broken(&|v| v["members"][0]["files"] = json!(null)), None);
    assert_eq!(
        broken(&|v| v["members"][0]["files"][0]["subdir"] = json!("win-64")),
        None
    );
    assert_eq!(
        broken(&|v| v["members"][0]["files"][0]["filename"] = json!(1)),
        None
    );
    assert_eq!(
        broken(&|v| v["members"][0]["files"][0]["sha256"] = json!("ABC")),
        None
    );
    assert_eq!(
        broken(&|v| v["members"][0]["files"][0]["size"] = json!("big")),
        None
    );
}

#[test]
fn an_open_group_holds_every_file_only_when_each_is_stored_with_the_declared_digest_and_size() {
    let members = set(&alpha());
    let both = ["linux-64", "noarch"];
    let read = |status: &str, stored: &[&str]| {
        parse_group(&bytes(&group_json(ID, status, &members, stored))).unwrap()
    };
    assert!(read("open", &both).holds_every_file(&members));
    assert!(!read("open", &["linux-64"]).holds_every_file(&members));
    assert!(!read("open", &[]).holds_every_file(&members));
    assert!(!read("validating", &both).holds_every_file(&members));
    // Another digest, another size or another member is not the declared set.
    let mut other = members.clone();
    other[0].files[0].sha256 = "f".repeat(64);
    assert!(!read("open", &both).holds_every_file(&other));
    let mut other = members.clone();
    other[0].files[0].size += 1;
    assert!(!read("open", &both).holds_every_file(&other));
    let mut other = members.clone();
    other[0].name = "beta".to_owned();
    assert!(!read("open", &both).holds_every_file(&other));
}

#[test]
fn a_create_answer_lists_what_to_put_and_what_is_stored() {
    let members = set(&alpha());
    let body = bytes(&created_json(ID, "open", &members, &["noarch"]));
    let created = parse_created(&body).unwrap();
    assert_eq!(created.id, ID);
    assert_eq!(created.status, GroupStatus::Open);
    assert!(!created.reserving);
    assert!(created.superseded.is_empty());
    let file = &members[0].files[0];
    let target = created.pending(&members[0], file).unwrap();
    assert_eq!(
        target.url.redacted(),
        "https://files.example.com/store/linux-64/mojo-alpha-1.0.0-mojo26.2_0.conda?<redacted>"
    );
    assert_eq!(
        target.headers,
        vec![("x-amz-checksum-sha256".to_owned(), "abc".to_owned())]
    );
    assert!(put_url("x").starts_with("https://files.example.com/"));
    // A stored file, a file the answer does not list and a member it does not list have no target.
    assert!(created.pending(&members[0], &members[0].files[1]).is_none());
    let mut other = members.clone();
    other[0].files[0].file_name = "mojo-alpha-1.0.0-mojo9.9_0.conda".to_owned();
    assert!(created.pending(&members[0], &other[0].files[0]).is_none());
    other[0].name = "beta".to_owned();
    assert!(created.pending(&other[0], file).is_none());
}

#[test]
fn a_create_answer_notes_reserving_and_the_groups_it_superseded() {
    let members = set(&alpha());
    let mut value = created_json(ID, "open", &members, &[]);
    value["reserving"] = json!({"done": 0, "total": 1});
    value["superseded"] = json!([OTHER]);
    let created = parse_created(&bytes(&value)).unwrap();
    assert!(created.reserving);
    assert_eq!(created.superseded, vec![OTHER.to_owned()]);
}

#[test]
fn a_create_body_with_a_wrong_or_missing_field_is_not_read() {
    let members = set(&alpha());
    let good = created_json(ID, "open", &members, &["noarch"]);
    assert!(parse_created(&bytes(&good)).is_some());
    assert!(parse_created(b"[]").is_none());
    let broken = |change: &dyn Fn(&mut Value)| {
        let mut value = good.clone();
        change(&mut value);
        parse_created(&bytes(&value)).is_none()
    };
    assert!(broken(&|v| v["group_id"] = json!("x")));
    assert!(broken(&|v| v["status"] = json!("finished")));
    assert!(broken(&|v| v["members"] = json!({})));
    assert!(broken(&|v| v["superseded"] = json!("grp")));
    assert!(broken(&|v| v["superseded"] = json!([1])));
    assert!(broken(&|v| v["members"][0]["name"] = json!(1)));
    assert!(broken(&|v| v["members"][0]["version"] = json!(1)));
    assert!(broken(&|v| v["members"][0]["files"] = json!(1)));
    assert!(broken(
        &|v| v["members"][0]["files"][0]["subdir"] = json!("win-64")
    ));
    assert!(broken(
        &|v| v["members"][0]["files"][0]["filename"] = json!(1)
    ));
    assert!(broken(
        &|v| v["members"][0]["files"][0]["uploaded"] = json!("yes")
    ));
    // A file that is not stored has a URL, and it is an https URL with string headers.
    assert!(broken(&|v| {
        v["members"][0]["files"][0]
            .as_object_mut()
            .unwrap()
            .remove("put_url");
    }));
    assert!(broken(
        &|v| v["members"][0]["files"][0]["put_url"] = json!("http://x.example/a")
    ));
    assert!(broken(
        &|v| v["members"][0]["files"][0]["put_headers"] = json!("none")
    ));
    assert!(broken(
        &|v| v["members"][0]["files"][0]["put_headers"] = json!({"a": 1})
    ));
    // No headers at all is allowed.
    let mut value = good.clone();
    value["members"][0]["files"][0]
        .as_object_mut()
        .unwrap()
        .remove("put_headers");
    let created = parse_created(&bytes(&value)).unwrap();
    let target = created.pending(&members[0], &members[0].files[0]).unwrap();
    assert!(target.headers.is_empty());
}

#[test]
fn a_version_record_is_read_with_its_state_files_and_group() {
    for (text, state) in [
        ("published", VersionState::Published),
        ("yanked", VersionState::Yanked),
        ("tombstoned", VersionState::Tombstoned),
    ] {
        let value = record_json("alpha", "1.0.0", text, &["linux-64", "noarch"]);
        let record = parse_record(&bytes(&value)).unwrap();
        assert_eq!(record.state, state);
        assert_eq!(record.name, "alpha");
        assert_eq!(record.version, "1.0.0");
        assert_eq!(record.files.len(), 2);
        assert_eq!(record.group_id, None);
        assert_eq!(record.files[0].url.is_some(), text != "tombstoned");
    }
    let mut value = record_json("alpha", "1.0.0", "published", &["linux-64"]);
    value["group_id"] = json!(ID);
    let record = parse_record(&bytes(&value)).unwrap();
    assert_eq!(record.group_id.as_deref(), Some(ID));
    // A burned number has no files.
    let burned = record_json("alpha", "1.0.0", "tombstoned", &[]);
    assert!(parse_record(&bytes(&burned)).unwrap().files.is_empty());
}

#[test]
fn a_version_record_with_a_wrong_field_or_two_files_for_a_subdirectory_is_not_read() {
    let good = record_json("alpha", "1.0.0", "published", &["linux-64", "noarch"]);
    assert!(parse_record(&bytes(&good)).is_some());
    assert!(parse_record(b"nope").is_none());
    let broken = |change: &dyn Fn(&mut Value)| {
        let mut value = good.clone();
        change(&mut value);
        parse_record(&bytes(&value)).is_none()
    };
    assert!(broken(&|v| v["state"] = json!("pending")));
    assert!(broken(&|v| v["name"] = json!(1)));
    assert!(broken(&|v| v["version"] = json!(1)));
    assert!(broken(&|v| v["files"] = json!("none")));
    assert!(broken(&|v| v["files"][0]["subdir"] = json!("win-64")));
    assert!(broken(&|v| v["files"][0]["sha256"] = json!("abc")));
    assert!(broken(&|v| v["files"][0]["size"] = json!(-1)));
    assert!(broken(&|v| v["files"][0]["filename"] = json!(1)));
    assert!(broken(&|v| v["group_id"] = json!("grp_x")));
    // Two files for one subdirectory.
    assert!(broken(&|v| v["files"][1]["subdir"] = json!("linux-64")));
}
