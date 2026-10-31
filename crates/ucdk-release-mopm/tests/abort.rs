// SPDX-License-Identifier: Apache-2.0
//! `abort` against a registry written by hand: the group is addressed by the declared set, and each
//! answer of mopm is read against the table of what an abort gives back.

mod common;

use std::time::Duration;

use common::*;
use ucdk_core::ManualClock;
use ucdk_release::codec::GrantFault;
use ucdk_release_mopm::{Caller, Member, Method, Registry, group_id};
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::SeamError;

fn abort_kind(result: &Result<(), SeamError>) -> &'static str {
    match result {
        Ok(()) => "ok",
        Err(error) => err_kind(error),
    }
}

fn abort_of(fake: Fake, members: &[Member]) -> (Result<(), SeamError>, Registry<Fake>) {
    let mut reg = registry(fake, &ManualClock::at(1_000_000));
    let result = reg.abort(&caller(), members);
    (result, reg)
}

#[test]
fn an_abort_by_the_staged_set_deletes_the_group_that_set_names() {
    let (_, members, id) = world();
    let aborted = group_json(&id, "aborted", &members, &[]);
    let fake = Fake::default().on(
        Method::Delete,
        &group_path(&id),
        vec![json_reply(200, &aborted)],
    );
    let (result, reg) = abort_of(fake, &members);
    assert_eq!(result, Ok(()));
    assert_eq!(
        reg.transport().lines(),
        [format!("DELETE {}", group_path(&id))]
    );
    assert!(reg.transport().log[0].authorized);
}

#[test]
fn an_abort_by_a_set_that_differs_in_one_digest_deletes_another_group_and_gives_nothing_back() {
    let (_, _, id) = world();
    let mut changed = alpha();
    changed[0].sha256 = "f".repeat(64);
    let other = set(&changed);
    let other_id = group_id(ACTOR, &other);
    assert_ne!(other_id, id);
    let fake = Fake::default()
        .on(Method::Delete, &group_path(&id), vec![ok(200)])
        .on(
            Method::Delete,
            &group_path(&other_id),
            vec![error_reply(404, "group_not_found")],
        )
        .on(
            Method::Get,
            &group_path(&other_id),
            vec![error_reply(404, "group_not_found")],
        );
    let (result, reg) = abort_of(fake, &other);
    assert_eq!(result, Ok(()));
    assert_eq!(
        reg.transport().lines(),
        [
            format!("DELETE {}", group_path(&other_id)),
            format!("GET {}", group_path(&other_id)),
        ]
    );
    // The staged set's own group was never named.
    assert_eq!(reg.transport().count(Method::Delete, &group_path(&id)), 0);
}

#[test]
fn an_abort_of_no_declared_items_sends_nothing() {
    let (result, reg) = abort_of(Fake::default(), &[]);
    assert_eq!(result, Ok(()));
    assert!(reg.transport().log.is_empty());
}

#[test]
fn a_group_that_is_not_open_is_read_and_gives_the_answer_of_its_status() {
    let (_, members, id) = world();
    let table = [
        ("expired", "ok"),
        ("rejected", "ok"),
        ("superseded", "ok"),
        ("aborted", "ok"),
        ("committed", "ok"),
        ("visible", "ok"),
        ("failed", "ok"),
        ("validating", "in_flight"),
        ("open", "unknown"),
    ];
    for (status, expected) in table {
        let read = group_json(&id, status, &members, &[]);
        let fake = Fake::default()
            .on(
                Method::Delete,
                &group_path(&id),
                vec![error_reply(409, "group_not_open")],
            )
            .on(Method::Get, &group_path(&id), vec![json_reply(200, &read)]);
        let (result, reg) = abort_of(fake, &members);
        assert_eq!(abort_kind(&result), expected, "{status}");
        assert_eq!(reg.transport().log.len(), 2, "{status}");
        assert!(reg.transport().log[1].authorized, "{status}");
    }
    // A group that is gone by the time it is read has nothing left to give back.
    let fake = Fake::default()
        .on(
            Method::Delete,
            &group_path(&id),
            vec![error_reply(409, "group_not_open")],
        )
        .on(
            Method::Get,
            &group_path(&id),
            vec![error_reply(404, "group_not_found")],
        );
    let (result, _) = abort_of(fake, &members);
    assert_eq!(result, Ok(()));
}

#[test]
fn a_404_is_ok_only_when_the_group_read_agrees_the_group_is_gone() {
    let (_, members, id) = world();
    let shown = json_reply(200, &group_json(&id, "aborted", &members, &[]));
    let table = [
        (error_reply(404, "group_not_found"), "ok"),
        (shown, "unknown"),
        (error_reply(500, "internal_error"), "unknown"),
        (lost(), "unknown"),
    ];
    for (read, expected) in table {
        let fake = Fake::default()
            .on(
                Method::Delete,
                &group_path(&id),
                vec![error_reply(404, "group_not_found")],
            )
            .on(Method::Get, &group_path(&id), vec![read]);
        let (result, _) = abort_of(fake, &members);
        assert_eq!(abort_kind(&result), expected);
    }
}

#[test]
fn the_other_answers_of_a_delete_are_classified_and_a_lost_one_is_unknown() {
    let (_, members, id) = world();
    let table = [
        (
            error_reply(401, "token_expired"),
            Err(SeamError::GrantRefused(GrantFault::Expired)),
        ),
        (
            error_reply(401, "token_unknown"),
            Err(SeamError::GrantRefused(GrantFault::Denied)),
        ),
        (
            error_reply(403, "forbidden"),
            Err(SeamError::GrantRefused(GrantFault::Denied)),
        ),
        (
            error_reply(404, "route_not_found"),
            Err(SeamError::Refused(Reason::Channel)),
        ),
    ];
    for (reply, expected) in table {
        let fake = Fake::default().on(Method::Delete, &group_path(&id), vec![reply]);
        let (result, reg) = abort_of(fake, &members);
        assert_eq!(result, expected);
        assert_eq!(reg.transport().log.len(), 1, "no read follows a refusal");
    }
    for reply in [
        error_reply(500, "internal_error"),
        error_reply(404, "package_not_found"),
        lost(),
    ] {
        let fake = Fake::default().on(Method::Delete, &group_path(&id), vec![reply]);
        let (result, _) = abort_of(fake, &members);
        assert_eq!(abort_kind(&result), "unknown");
    }
}

#[test]
fn an_abort_out_of_budget_after_the_delete_does_not_read_the_group() {
    let (_, members, id) = world();
    let fake = Fake::default().on(
        Method::Delete,
        &group_path(&id),
        vec![error_reply(409, "group_not_open")],
    );
    let mut cfg = group_config();
    cfg.abort_timeout = Duration::ZERO;
    let mut reg = Registry::new(fake, cfg, Box::new(ManualClock::at(1_000_000)));
    let result = reg.abort(&caller(), &members);
    assert_eq!(abort_kind(&result), "unknown");
    assert_eq!(reg.transport().log.len(), 1);
}

#[test]
fn a_token_that_cannot_be_sent_is_refused_and_nothing_is_sent() {
    let (_, members, _) = world();
    let spaced = Caller {
        actor: ACTOR,
        token: b"has a space",
    };
    let mut reg = registry(Fake::default(), &ManualClock::at(1_000_000));
    let result = reg.abort(&spaced, &members);
    assert_eq!(result, Err(SeamError::GrantRefused(GrantFault::Denied)));
    assert!(reg.transport().log.is_empty());
}
