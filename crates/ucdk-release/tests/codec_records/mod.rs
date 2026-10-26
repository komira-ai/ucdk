// SPDX-License-Identifier: Apache-2.0
//! One record of each kind, in the forms the codec tests write, read and break.

#![allow(dead_code)]

use ucdk_core::Hash256;
use ucdk_release::codec::{
    Action, Encoded, Grant, GrantAnswer, GrantFault, GrantPhase, Phase, Publication, Record, Run,
    Seal, Stage, Validation,
};

pub fn hash(c: char) -> Hash256 {
    Hash256::from_hex(&c.to_string().repeat(64)).expect("a hash")
}

pub fn run() -> Record {
    Record::Run(Run {
        run_id: "run-7".to_owned(),
        machine: hash('1'),
        revision: "0123456789abcdef".to_owned(),
        version: "1.2.3".to_owned(),
        build: 41,
        commit: "abc123".to_owned(),
        timestamp: 1_700_000_000_000,
    })
}

pub fn run_of(record: Record) -> Run {
    let Record::Run(r) = record else {
        panic!("a run")
    };
    r
}

pub fn stage_of(record: Record) -> Stage {
    let Record::Stage(s) = record else {
        panic!("a stage")
    };
    s
}

pub fn stage(full: bool) -> Record {
    Record::Stage(Stage {
        stage: "build".to_owned(),
        attempt: 2,
        platform: "linux-x86_64".to_owned(),
        set: full.then(|| hash('2')),
        generation: full.then(|| "run-7-a2".to_owned()),
        confirmed: full,
    })
}

pub fn action_of(record: Record) -> Action {
    let Record::Action(a) = record else {
        panic!("an action")
    };
    a
}

pub fn action(full: bool) -> Record {
    Record::Action(Action {
        stage: "build".to_owned(),
        action: "compile".to_owned(),
        input: hash('3'),
        set: full.then(|| hash('2')),
        generation: full.then(|| "run-7-a2".to_owned()),
    })
}

pub fn publication_of(record: Record) -> Publication {
    let Record::Publication(p) = record else {
        panic!("a publication")
    };
    p
}

pub fn publication(exceptional: bool, phase: Option<Phase>) -> Record {
    Record::Publication(Publication {
        coordinate: "stable/file/tool/1.2.3+build.4/linux-x86_64".to_owned(),
        digest: hash('4'),
        build: 41,
        commit: "abc123".to_owned(),
        exceptional,
        run: "run-7".to_owned(),
        stage: "publish".to_owned(),
        attempt: 3,
        phase,
    })
}

pub fn validation_of(record: Record) -> Validation {
    let Record::Validation(v) = record else {
        panic!("a validation")
    };
    v
}

pub fn validation(full: bool) -> Record {
    Record::Validation(Validation {
        stage: "build".to_owned(),
        validation: "closure".to_owned(),
        set: full.then(|| hash('2')),
    })
}

pub fn grant_of(record: Record) -> Grant {
    let Record::Grant(g) = record else {
        panic!("a grant")
    };
    g
}

pub fn grant_with(
    phase: GrantPhase,
    holder: &str,
    answer: Option<GrantAnswer>,
    attempt: u32,
    n: u32,
) -> Record {
    Record::Grant(Grant {
        stage: "publish".to_owned(),
        holder: holder.to_owned(),
        phase,
        attempt,
        n,
        channel: "stable".to_owned(),
        repository: "file".to_owned(),
        subject: "repo:example/project:ref:refs/heads/main".to_owned(),
        audience: "registry".to_owned(),
        scope: "stage".to_owned(),
        coordinates: hash('6'),
        answer,
    })
}

/// A grant with every field set, granted.
pub fn grant() -> Record {
    grant_with(
        GrantPhase::Stage,
        "push",
        Some(GrantAnswer::Granted {
            expires_ms: 1_700_000_600_000,
        }),
        2,
        1,
    )
}

pub fn seal_of(record: Record) -> Seal {
    let Record::Seal(s) = record else {
        panic!("a seal")
    };
    s
}

pub fn seal() -> Record {
    Record::Seal(Seal {
        stage: "build".to_owned(),
        platform: "linux-x86_64".to_owned(),
        revision: "0123456789abcdef".to_owned(),
        set: hash('2'),
        generation: "run-7-a2".to_owned(),
        inputs: hash('5'),
        environment: "build".to_owned(),
        machine: "demo".to_owned(),
        version: "1.2.3".to_owned(),
        build: 41,
        commit: "abc123".to_owned(),
        timestamp: 1_700_000_000_000,
        run: "run-7".to_owned(),
        attempt: 2,
    })
}

/// One of each kind, fully written, and the lighter forms of those that have optional keys.
pub fn every_record() -> Vec<Record> {
    let mut all = vec![
        run(),
        stage(true),
        stage(false),
        action(true),
        action(false),
        publication(true, Some(Phase::Staging)),
        publication(true, Some(Phase::Committing)),
        publication(false, None),
        validation(true),
        validation(false),
        grant(),
        grant_with(GrantPhase::Commit, "push", None, 1, 1),
        grant_with(
            GrantPhase::Read,
            "validation:readback",
            Some(GrantAnswer::Refused {
                fault: GrantFault::Denied,
            }),
            4,
            12,
        ),
        seal(),
    ];
    for fault in [
        GrantFault::Denied,
        GrantFault::Expired,
        GrantFault::Used,
        GrantFault::Unreachable,
    ] {
        all.push(grant_with(
            GrantPhase::Commit,
            "push",
            Some(GrantAnswer::Refused { fault }),
            1,
            0,
        ));
    }
    all
}

/// The record as the codec writes it.
pub fn written(record: &Record) -> Encoded {
    record.encode().expect("a record the codec writes")
}
