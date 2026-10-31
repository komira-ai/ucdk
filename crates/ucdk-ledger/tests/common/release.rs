// SPDX-License-Identifier: Apache-2.0
//! Release records in the ledger: a record of the release codec put into the fields a ledger
//! record has, written, and read back. The codec owns the tag, the readable name and the payload;
//! the ledger keeps them as it keeps any record, so these helpers add no field and no format.

use std::collections::BTreeSet;

use ucdk_core::{Hash256, RecordPath};
use ucdk_ledger::{Change, Ledger, Origin, OwnerPath, Record, State};
use ucdk_release::codec::{
    Encoded, Grant, GrantAnswer, GrantFault, GrantPhase, Phase, Publication, Record as Release,
    Run, Seal, coordinate_text,
};
use ucdk_release_proto::v1::{ArtifactType, Coordinate};

/// The ledger record that holds `release`, in `state` and `origin`. The record's path, kind,
/// name and payload are the codec's; it has no locator and no dependency.
pub fn ledger_record(release: &Release, origin: Origin, state: State) -> Record {
    let encoded = release.encode().expect("a record the codec writes");
    let path = encoded.path().expect("a path of valid segments");
    Record {
        owner: OwnerPath::of(&path),
        path,
        kind: encoded.kind,
        name: encoded.name,
        origin,
        state,
        physical: encoded.physical,
        deps: BTreeSet::new(),
        outputs: encoded.outputs,
    }
}

/// The release record a ledger record holds, read back through the codec.
pub fn release_record(record: &Record) -> Release {
    Release::decode(&Encoded {
        kind: record.kind.clone(),
        name: record.name.clone(),
        physical: record.physical.clone(),
        outputs: record.outputs.clone(),
    })
    .expect("a record the codec reads")
}

/// Writes `release` to `ledger` against the current token, and returns its path.
pub fn put_release(
    ledger: &dyn Ledger,
    release: &Release,
    origin: Origin,
    state: State,
) -> RecordPath {
    let record = ledger_record(release, origin, state);
    let path = record.path.clone();
    let token = ledger.read().expect("read").token;
    ledger
        .write(&token, Change::Put(record))
        .expect("write a release record");
    path
}

/// The release record at `path`, with the origin and state it is kept in.
pub fn read_release(ledger: &dyn Ledger, path: &RecordPath) -> Option<(Release, Origin, State)> {
    let snapshot = ledger.read().expect("read");
    let record = snapshot.records.get(path)?;
    assert_eq!(
        record.path,
        ledger_record(&release_record(record), record.origin, record.state).path,
        "a release record is kept at the path its codec gives it"
    );
    Some((release_record(record), record.origin, record.state))
}

fn hash(c: char) -> Hash256 {
    Hash256::from_hex(&c.to_string().repeat(64)).expect("a hash")
}

pub fn run() -> Release {
    Release::Run(Run {
        run_id: "run-7".to_owned(),
        machine: hash('1'),
        revision: "0123456789abcdef".to_owned(),
        version: "1.2.3".to_owned(),
        build: 41,
        commit: "abc123".to_owned(),
        timestamp: 1_700_000_000_000,
    })
}

/// The coordinate `tool` at `version` on `stable`, as the text a publication record keeps.
pub fn tool(version: &str) -> String {
    coordinate_text(&Coordinate {
        channel: "stable".to_owned(),
        r#type: ArtifactType::File as i32,
        name: "tool".to_owned(),
        version: version.to_owned(),
        platform: "linux-x86_64".to_owned(),
    })
}

pub fn publication(phase: Option<Phase>) -> Release {
    Release::Publication(Publication {
        coordinate: tool("1.2.3+build.4"),
        digest: hash('4'),
        build: 41,
        commit: "abc123".to_owned(),
        exceptional: false,
        run: "run-7".to_owned(),
        stage: "publish".to_owned(),
        attempt: 3,
        phase,
    })
}

pub fn seal() -> Release {
    Release::Seal(Seal {
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

pub fn grant(n: u32, answer: Option<GrantAnswer>) -> Release {
    Release::Grant(Grant {
        stage: "publish".to_owned(),
        holder: "push".to_owned(),
        phase: GrantPhase::Commit,
        attempt: 1,
        n,
        channel: "stable".to_owned(),
        repository: "file".to_owned(),
        subject: "repo:example/project:ref:refs/heads/main".to_owned(),
        audience: "registry".to_owned(),
        scope: "commit".to_owned(),
        coordinates: hash('6'),
        answer,
    })
}

pub fn granted() -> GrantAnswer {
    GrantAnswer::Granted {
        expires_ms: 1_700_000_300_000,
    }
}

pub fn refused() -> GrantAnswer {
    GrantAnswer::Refused {
        fault: GrantFault::Denied,
    }
}

/// The records the release golden holds, each with the origin and state it is kept in: a run, a
/// publication whose commit may have landed, a publication found in a registry, a seal created
/// confirmed in one write, and a grant asked, granted and refused.
pub fn golden_records() -> Vec<(Release, Origin, State)> {
    vec![
        (run(), Origin::Created, State::Confirmed),
        (
            publication(Some(Phase::Committing)),
            Origin::Created,
            State::Intent,
        ),
        (
            match publication(None) {
                Release::Publication(p) => Release::Publication(Publication {
                    coordinate: tool("1.2.2"),
                    ..p
                }),
                other => other,
            },
            Origin::Adopted,
            State::Confirmed,
        ),
        (seal(), Origin::Created, State::Confirmed),
        (grant(1, None), Origin::Created, State::Intent),
        (grant(2, Some(granted())), Origin::Created, State::Confirmed),
        (grant(3, Some(refused())), Origin::Created, State::Confirmed),
    ]
}
