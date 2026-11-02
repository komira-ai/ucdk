// SPDX-License-Identifier: Apache-2.0
//! A declared set of several packages, staged and aborted by a caller other than the default one:
//! the group is a function of the actor and the set (not of the order the set is given in), and
//! every group call carries the caller's own token.

mod common;

use common::*;
use serde_json::Value;
use ucdk_core::ManualClock;
use ucdk_release_mopm::{Caller, Member, Method, Registry, group_id};
use ucdk_release_seams::StageAnswer;

const OTHER_ACTOR: &str = "ci:gh-repo:987654";
const OTHER_TOKEN: &[u8] = b"OTHER-token-4567";

fn other() -> Caller<'static> {
    Caller {
        actor: OTHER_ACTOR,
        token: OTHER_TOKEN,
    }
}

fn two_packages() -> Vec<Owned> {
    let mut owned = alpha();
    owned.push(Owned::new("beta", "2.0.0", "any"));
    owned
}

fn declaring_both(fake: Fake) -> Registry<Fake> {
    let mut reg = Registry::new(fake, group_config(), Box::new(ManualClock::at(1_000_000)));
    reg.declare(&["alpha".to_owned(), "beta".to_owned()])
        .unwrap();
    reg
}

fn beta_put_path() -> String {
    format!("/store/noarch/{}", file_name("beta", "2.0.0"))
}

fn abort_path_of(members: &[Member], caller: &Caller<'_>) -> String {
    let id = group_id(caller.actor, members);
    let fake = Fake::default().on(Method::Delete, &group_path(&id), vec![ok(200)]);
    let mut reg = declaring_both(fake);
    assert_eq!(reg.abort(caller, members), Ok(()));
    let lines = reg.transport().lines();
    assert_eq!(lines.len(), 1);
    lines[0].clone()
}

#[test]
fn a_two_package_set_is_staged_in_the_group_of_its_actor_with_that_actors_token_and_aborted_there()
{
    let owned = two_packages();
    let members = set(&owned);
    let id = group_id(OTHER_ACTOR, &members);
    assert_ne!(id, group_id(ACTOR, &members));
    let created = json_reply(201, &created_json(&id, "open", &members, &[]));
    let fake = Fake::default()
        .on(
            Method::Get,
            &group_path(&id),
            vec![error_reply(404, "group_not_found")],
        )
        .on(Method::Post, GROUPS, vec![created])
        .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
        .on(Method::Put, &put_path("noarch"), vec![ok(200)])
        .on(Method::Put, &beta_put_path(), vec![ok(200)])
        .on(Method::Delete, &group_path(&id), vec![ok(200)]);
    let bytes = file_bytes();
    let files = set_files(&owned, &bytes);
    let mut reg = declaring_both(fake);
    assert_eq!(
        reg.stage(&other(), &members, &files),
        Ok(StageAnswer::Staged)
    );
    assert_eq!(
        reg.transport().lines(),
        [
            format!("GET {}", group_path(&id)),
            format!("POST {GROUPS}"),
            format!("PUT {}", put_path("linux-64")),
            format!("PUT {}", put_path("noarch")),
            format!("PUT {}", beta_put_path()),
        ]
    );
    let bearer = Some("Bearer OTHER-token-4567".to_owned());
    let log = &reg.transport().log;
    assert_eq!((&log[0].bearer, &log[1].bearer), (&bearer, &bearer));
    assert!(log[2..].iter().all(|l| l.bearer.is_none() && !l.authorized));
    let sent: Value = serde_json::from_slice(&log[1].raw).unwrap();
    let declared: Vec<(&str, &str)> = sent["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["name"].as_str().unwrap(), m["version"].as_str().unwrap()))
        .collect();
    assert_eq!(declared, [("alpha", "1.0.0"), ("beta", "2.0.0")]);
    assert_eq!(reg.abort(&other(), &members), Ok(()));
    let after = reg.transport();
    assert_eq!(
        after.lines().last().unwrap(),
        &format!("DELETE {}", group_path(&id))
    );
    assert_eq!(after.log.last().unwrap().bearer, bearer);
    assert_eq!(after.log.len(), 6);
}

#[test]
fn the_order_a_set_is_given_in_does_not_change_the_group_it_addresses() {
    let members = set(&two_packages());
    let mut reversed: Vec<Member> = members.iter().rev().cloned().collect();
    for member in &mut reversed {
        member.files.reverse();
    }
    assert_ne!(members, reversed);
    assert_eq!(
        group_id(OTHER_ACTOR, &members),
        group_id(OTHER_ACTOR, &reversed)
    );
    let id = group_id(OTHER_ACTOR, &members);
    let expected = format!("DELETE {}", group_path(&id));
    assert_eq!(abort_path_of(&members, &other()), expected);
    assert_eq!(abort_path_of(&reversed, &other()), expected);
}

#[test]
fn another_set_or_another_actor_aborts_another_group() {
    let members = set(&two_packages());
    let id = group_id(OTHER_ACTOR, &members);
    let full = abort_path_of(&members, &other());
    assert_eq!(full, format!("DELETE {}", group_path(&id)));

    let without_beta = set(&alpha());
    let alone = abort_path_of(&without_beta, &other());
    assert_eq!(
        alone,
        format!(
            "DELETE {}",
            group_path(&group_id(OTHER_ACTOR, &without_beta))
        )
    );
    assert_ne!(alone, full);

    let mut newer = two_packages();
    newer[2] = Owned::new("beta", "2.1.0", "any");
    let newer = set(&newer);
    let versioned = abort_path_of(&newer, &other());
    assert_eq!(
        versioned,
        format!("DELETE {}", group_path(&group_id(OTHER_ACTOR, &newer)))
    );
    assert_ne!(versioned, full);

    let by_default = abort_path_of(&members, &caller());
    assert_eq!(
        by_default,
        format!("DELETE {}", group_path(&group_id(ACTOR, &members)))
    );
    assert_ne!(by_default, full);
}
