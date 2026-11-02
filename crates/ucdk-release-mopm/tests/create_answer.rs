// SPDX-License-Identifier: Apache-2.0
//! A finished create answer must answer the declared set, an empty set is never staged, and the
//! reads that follow a `version_exists` or `number_taken` conflict stop when the stage budget is
//! spent.

mod common;

use common::*;
use serde_json::{Value, json};
use ucdk_core::ManualClock;
use ucdk_release_mopm::{Member, Method, Registry, SetFile};
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{SeamError, StageAnswer};

const START: u64 = 1_000_000;
const MALFORMED: &str = "malformed answer: a group create that does not answer the declared set";
const BUDGET: &str =
    "the call ran out of its time budget; the group stays open and the next stage resumes it";

fn malformed() -> Result<StageAnswer, SeamError> {
    Err(SeamError::Permanent(MALFORMED.into()))
}

fn no_group(id: &str) -> Fake {
    Fake::default().on(
        Method::Get,
        &group_path(id),
        vec![error_reply(404, "group_not_found")],
    )
}

fn stage_with(
    mut reg: Registry<Fake>,
    owned: &[Owned],
    members: &[Member],
) -> (Result<StageAnswer, SeamError>, Registry<Fake>) {
    let bytes = file_bytes();
    let files: Vec<SetFile<'_>> = set_files(owned, &bytes);
    let result = reg.stage(&caller(), members, &files);
    (result, reg)
}

fn stage_of(
    fake: Fake,
    owned: &[Owned],
    members: &[Member],
) -> (Result<StageAnswer, SeamError>, Registry<Fake>) {
    let reg = registry(fake, &ManualClock::at(START));
    stage_with(reg, owned, members)
}

fn open_answer(id: &str, members: &[Member]) -> Value {
    created_json(id, "open", members, &[])
}

fn files_of(answer: &mut Value, member: usize) -> &mut Vec<Value> {
    answer["members"][member]["files"].as_array_mut().unwrap()
}

fn members_of(answer: &mut Value) -> &mut Vec<Value> {
    answer["members"].as_array_mut().unwrap()
}

fn assert_malformed_and_nothing_put(answer: &Value, what: &str) {
    let (owned, members, id) = world();
    let fake = no_group(&id).on(Method::Post, GROUPS, vec![json_reply(201, answer)]);
    let (result, reg) = stage_of(fake, &owned, &members);
    assert_eq!(result, malformed(), "{what}");
    assert_eq!(
        reg.transport().lines(),
        [format!("GET {}", group_path(&id)), format!("POST {GROUPS}")],
        "{what}"
    );
}

#[test]
fn an_open_create_answer_with_no_members_is_malformed_and_is_not_taken_for_a_duplicate() {
    let (_, members, id) = world();
    let mut answer = open_answer(&id, &members);
    members_of(&mut answer).clear();
    assert_malformed_and_nothing_put(&answer, "members: []");
}

#[test]
fn an_open_create_answer_that_omits_a_declared_file_is_malformed() {
    let (_, members, id) = world();
    let mut answer = open_answer(&id, &members);
    files_of(&mut answer, 0).pop();
    assert_malformed_and_nothing_put(&answer, "one file omitted");
}

#[test]
fn an_open_create_answer_for_another_version_is_malformed() {
    let (_, members, id) = world();
    let mut answer = open_answer(&id, &members);
    answer["members"][0]["version"] = json!("1.0.1");
    assert_malformed_and_nothing_put(&answer, "another version");
}

#[test]
fn an_open_create_answer_with_another_filename_is_malformed() {
    let (_, members, id) = world();
    let mut answer = open_answer(&id, &members);
    answer["members"][0]["files"][0]["filename"] = json!("other-1.0.0-0.conda");
    assert_malformed_and_nothing_put(&answer, "another filename");
}

#[test]
fn an_open_create_answer_that_lists_a_member_twice_is_malformed() {
    let (_, members, id) = world();
    let mut answer = open_answer(&id, &members);
    let again = answer["members"][0].clone();
    members_of(&mut answer).push(again);
    assert_malformed_and_nothing_put(&answer, "member twice");
}

#[test]
fn an_open_create_answer_that_lists_a_file_twice_is_malformed() {
    let (_, members, id) = world();
    let mut answer = open_answer(&id, &members);
    let again = answer["members"][0]["files"][0].clone();
    files_of(&mut answer, 0).push(again);
    assert_malformed_and_nothing_put(&answer, "file twice");
}

#[test]
fn an_open_create_answer_with_an_undeclared_extra_member_is_malformed() {
    let (_, members, id) = world();
    let mut answer = open_answer(&id, &members);
    members_of(&mut answer)
        .push(json!({"name": "beta", "version": "2.0.0", "action": "new", "files": []}));
    assert_malformed_and_nothing_put(&answer, "extra member");
}

#[test]
fn a_create_answer_in_a_status_a_create_never_answers_is_malformed() {
    let (_, members, id) = world();
    for status in ["rejected", "failed", "expired", "aborted", "superseded"] {
        let answer = created_json(&id, status, &members, &[]);
        assert_malformed_and_nothing_put(&answer, status);
    }
}

#[test]
fn a_create_answer_past_open_that_lists_files_is_malformed() {
    let (_, members, id) = world();
    let mut answer = open_answer(&id, &members);
    answer["status"] = json!("validating");
    assert_malformed_and_nothing_put(&answer, "validating with files");
}

#[test]
fn a_create_answer_past_open_that_lists_a_member_twice_is_malformed() {
    let (_, members, id) = world();
    let mut answer = created_json(&id, "validating", &members, &[]);
    let member = answer["members"][0].clone();
    members_of(&mut answer).push(member);
    assert_malformed_and_nothing_put(&answer, "validating with a member twice");
}

#[test]
fn a_create_answer_past_open_that_lists_the_declared_members_without_files_is_a_duplicate() {
    let (owned, members, id) = world();
    for status in ["validating", "committed", "visible"] {
        let answer = created_json(&id, status, &members, &[]);
        let fake = no_group(&id).on(Method::Post, GROUPS, vec![json_reply(201, &answer)]);
        let (result, reg) = stage_of(fake, &owned, &members);
        assert_eq!(result, Ok(StageAnswer::Duplicate), "{status}");
        assert_eq!(reg.transport().count(Method::Put, &put_path("noarch")), 0);
    }
}

#[test]
fn a_refresh_create_is_held_to_the_declared_set_too() {
    let (owned, members, id) = world();
    let good = open_answer(&id, &members);
    let mut empty = open_answer(&id, &members);
    members_of(&mut empty).clear();
    let fake = no_group(&id)
        .on(
            Method::Post,
            GROUPS,
            vec![json_reply(201, &good), json_reply(201, &empty)],
        )
        .on(Method::Put, &put_path("linux-64"), vec![ok(403)])
        .on(Method::Put, &put_path("noarch"), vec![ok(200)]);
    let (result, reg) = stage_of(fake, &owned, &members);
    assert_eq!(result, malformed());
    assert_eq!(
        reg.transport().lines(),
        [
            format!("GET {}", group_path(&id)),
            format!("POST {GROUPS}"),
            format!("PUT {}", put_path("linux-64")),
            format!("POST {GROUPS}"),
        ]
    );
}

#[test]
fn an_answer_that_lists_the_members_and_files_in_another_order_still_uploads() {
    let (owned, members, id) = world();
    let mut answer = open_answer(&id, &members);
    files_of(&mut answer, 0).reverse();
    let fake = no_group(&id)
        .on(Method::Post, GROUPS, vec![json_reply(201, &answer)])
        .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
        .on(Method::Put, &put_path("noarch"), vec![ok(200)]);
    let (result, reg) = stage_of(fake, &owned, &members);
    assert_eq!(result, Ok(StageAnswer::Staged));
    assert_eq!(reg.transport().count(Method::Put, &put_path("linux-64")), 1);
    assert_eq!(reg.transport().count(Method::Put, &put_path("noarch")), 1);
}

#[test]
fn an_answer_that_lists_two_members_in_another_order_than_declared_still_uploads() {
    let (owned, members, id) = two_members();
    let mut answer = open_answer(&id, &members);
    members_of(&mut answer).reverse();
    let fake = no_group(&id)
        .on(Method::Post, GROUPS, vec![json_reply(201, &answer)])
        .on(
            Method::Put,
            &put_path_of("alpha", "1.0.0", "linux-64"),
            vec![ok(200)],
        )
        .on(
            Method::Put,
            &put_path_of("alpha", "1.0.0", "noarch"),
            vec![ok(200)],
        )
        .on(
            Method::Put,
            &put_path_of("beta", "2.0.0", "noarch"),
            vec![ok(200)],
        );
    let (result, reg) = stage_with(
        two_name_registry(fake, &ManualClock::at(START)),
        &owned,
        &members,
    );
    assert_eq!(result, Ok(StageAnswer::Staged));
    assert_eq!(reg.transport().log.len(), 5);
}

#[test]
fn an_empty_set_is_permanent_and_nothing_is_sent_even_to_a_registry_told_no_names() {
    let reg = Registry::new(
        Fake::default(),
        group_config(),
        Box::new(ManualClock::at(START)),
    );
    let (result, reg) = stage_with(reg, &[], &[]);
    assert_eq!(
        result,
        Err(SeamError::Permanent(
            "an empty set has nothing to stage".into()
        ))
    );
    assert!(reg.transport().log.is_empty());
}

fn put_path_of(name: &str, version: &str, subdir: &str) -> String {
    format!("/store/{subdir}/{}", file_name(name, version))
}

/// `alpha 1.0.0` with a file for each of two subdirectories, and `beta 2.0.0` with one.
fn two_members() -> (Vec<Owned>, Vec<Member>, String) {
    let mut owned = alpha();
    owned.push(Owned::new("beta", "2.0.0", "any"));
    let members = set(&owned);
    let id = ucdk_release_mopm::group_id(ACTOR, &members);
    (owned, members, id)
}

fn two_name_registry(fake: Fake, clock: &ManualClock) -> Registry<Fake> {
    let mut reg = Registry::new(fake, group_config(), Box::new(clock.clone()));
    reg.declare(&["alpha".to_owned(), "beta".to_owned()])
        .unwrap();
    reg
}

fn record_paths(members: &[Member]) -> Vec<String> {
    members
        .iter()
        .map(|m| record_path(&m.name, &m.version))
        .collect()
}

fn version_exists_world(
    alpha_record: Reply,
    beta_record: Reply,
) -> (Result<StageAnswer, SeamError>, Vec<String>, Vec<String>) {
    let (owned, members, id) = two_members();
    let fake = no_group(&id)
        .on(
            Method::Post,
            GROUPS,
            vec![error_reply(409, "version_exists")],
        )
        .on(
            Method::Get,
            &record_path("alpha", "1.0.0"),
            vec![alpha_record],
        )
        .on(
            Method::Get,
            &record_path("beta", "2.0.0"),
            vec![beta_record],
        );
    let (result, reg) = stage_with(
        two_name_registry(fake, &ManualClock::at(START)),
        &owned,
        &members,
    );
    let mut expected = vec![format!("GET {}", group_path(&id)), format!("POST {GROUPS}")];
    expected.extend(record_paths(&members).iter().map(|p| format!("GET {p}")));
    (result, reg.transport().lines(), expected)
}

#[test]
fn version_exists_with_a_sealed_member_missing_a_file_is_refused_even_when_another_has_no_record() {
    let held = json_reply(
        200,
        &record_json("alpha", "1.0.0", "published", &["noarch"]),
    );
    let (result, lines, expected) =
        version_exists_world(held, error_reply(404, "version_not_found"));
    assert_eq!(result, Err(SeamError::Refused(Reason::Channel)));
    assert_eq!(lines, expected);
}

#[test]
fn version_exists_with_a_yanked_member_missing_a_file_and_an_absent_one_is_refused() {
    let held = json_reply(200, &record_json("alpha", "1.0.0", "yanked", &["linux-64"]));
    let (result, lines, expected) =
        version_exists_world(held, error_reply(404, "version_not_found"));
    assert_eq!(result, Err(SeamError::Refused(Reason::Channel)));
    assert_eq!(lines, expected);
}

#[test]
fn version_exists_with_every_file_held_for_one_member_and_the_other_absent_is_transient() {
    let held = json_reply(
        200,
        &record_json("alpha", "1.0.0", "published", &["linux-64", "noarch"]),
    );
    let (result, lines, expected) =
        version_exists_world(held, error_reply(404, "version_not_found"));
    assert_eq!(
        result,
        Err(SeamError::Transient(
            "mopm says the version exists but its record cannot be read".into()
        ))
    );
    assert_eq!(lines, expected);
}

#[test]
fn version_exists_with_every_file_of_every_member_held_is_duplicate() {
    let alpha_held = json_reply(
        200,
        &record_json("alpha", "1.0.0", "published", &["linux-64", "noarch"]),
    );
    let beta_held = json_reply(200, &record_json("beta", "2.0.0", "published", &["noarch"]));
    let (result, lines, expected) = version_exists_world(alpha_held, beta_held);
    assert_eq!(result, Ok(StageAnswer::Duplicate));
    assert_eq!(lines, expected);
}

fn spent() -> Result<StageAnswer, SeamError> {
    Err(SeamError::Transient(BUDGET.into()))
}

#[test]
fn a_create_that_spends_the_stage_budget_before_version_exists_is_read_gives_no_record_read() {
    let (owned, members, id) = world();
    let clock = ManualClock::at(START);
    let fake = no_group(&id)
        .on(
            Method::Post,
            GROUPS,
            vec![error_reply(409, "version_exists")],
        )
        .bump(Method::Post, GROUPS, &clock, secs(300))
        .on(
            Method::Get,
            &record_path("alpha", "1.0.0"),
            vec![json_reply(
                200,
                &record_json("alpha", "1.0.0", "published", &["linux-64", "noarch"]),
            )],
        );
    let (result, reg) = stage_with(
        declared_registry(fake, group_config(), &clock),
        &owned,
        &members,
    );
    assert_eq!(result, spent());
    assert_eq!(
        reg.transport().lines(),
        [format!("GET {}", group_path(&id)), format!("POST {GROUPS}")]
    );
}

#[test]
fn a_record_read_that_spends_the_stage_budget_stops_the_reads_of_the_other_members() {
    let (owned, members, id) = two_members();
    let clock = ManualClock::at(START);
    let paths = record_paths(&members);
    let held = |m: &Member| {
        let subdirs: Vec<&str> = m.files.iter().map(|f| f.subdir.as_str()).collect();
        json_reply(
            200,
            &record_json(&m.name, &m.version, "published", &subdirs),
        )
    };
    let fake = no_group(&id)
        .on(
            Method::Post,
            GROUPS,
            vec![error_reply(409, "version_exists")],
        )
        .on(Method::Get, &paths[0], vec![held(&members[0])])
        .bump(Method::Get, &paths[0], &clock, secs(300))
        .on(Method::Get, &paths[1], vec![held(&members[1])]);
    let (result, reg) = stage_with(two_name_registry(fake, &clock), &owned, &members);
    assert_eq!(result, spent());
    assert_eq!(
        reg.transport().lines(),
        [
            format!("GET {}", group_path(&id)),
            format!("POST {GROUPS}"),
            format!("GET {}", paths[0]),
        ]
    );
}

#[test]
fn number_taken_reads_no_record_once_the_create_has_spent_the_stage_budget() {
    let (owned, members, id) = world();
    let clock = ManualClock::at(START);
    let fake = no_group(&id)
        .on(
            Method::Post,
            GROUPS,
            vec![conflict_reply(Some("number_taken"))],
        )
        .bump(Method::Post, GROUPS, &clock, secs(300))
        .on(
            Method::Get,
            &record_path("alpha", "1.0.0"),
            vec![error_reply(404, "version_not_found")],
        );
    let (result, reg) = stage_with(
        declared_registry(fake, group_config(), &clock),
        &owned,
        &members,
    );
    assert_eq!(result, spent());
    assert_eq!(
        reg.transport().lines(),
        [format!("GET {}", group_path(&id)), format!("POST {GROUPS}")]
    );
}

#[test]
fn number_taken_stops_reading_records_once_a_read_has_spent_the_stage_budget() {
    let (owned, members, id) = two_members();
    let clock = ManualClock::at(START);
    let paths = record_paths(&members);
    let fake = no_group(&id)
        .on(
            Method::Post,
            GROUPS,
            vec![conflict_reply(Some("number_taken"))],
        )
        .on(
            Method::Get,
            &paths[0],
            vec![error_reply(404, "version_not_found")],
        )
        .bump(Method::Get, &paths[0], &clock, secs(300))
        .on(
            Method::Get,
            &paths[1],
            vec![error_reply(404, "version_not_found")],
        );
    let (result, reg) = stage_with(two_name_registry(fake, &clock), &owned, &members);
    assert_eq!(result, spent());
    assert_eq!(
        reg.transport().lines(),
        [
            format!("GET {}", group_path(&id)),
            format!("POST {GROUPS}"),
            format!("GET {}", paths[0]),
        ]
    );
}
