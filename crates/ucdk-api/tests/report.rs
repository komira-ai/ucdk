// SPDX-License-Identifier: Apache-2.0
//! The shared validator of stage reports and run reports: what it accepts, and each thing it
//! refuses.

use ucdk_api::v1::ExitClass;
use ucdk_api::{
    ReportError as E, default_retry, exit_class, report_schema, validate_run, validate_stage,
};
use ucdk_release::run_identity::RunIdentityError;
use ucdk_release_proto::v1::{
    ActionReport, Effect, EffectState, Reason, Retry, Row, RowState, RunIdentity, RunReport,
    StageReport, ValidationReport, Verdict,
};

const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";
const SET_HASH: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn run() -> RunIdentity {
    RunIdentity {
        run_id: "gh-42".to_owned(),
        attempt: 1,
        context: [("actor".to_owned(), "ci".to_owned())].into(),
    }
}

fn effect(subject: &str, state: EffectState) -> Effect {
    Effect {
        what: "publish".to_owned(),
        subject: subject.to_owned(),
        state: state.into(),
    }
}

fn action(name: &str) -> ActionReport {
    ActionReport {
        name: name.to_owned(),
        verdict: Verdict::Succeeded.into(),
        ..ActionReport::default()
    }
}

/// A finished stage report that ended well.
fn stage() -> StageReport {
    StageReport {
        schema_version: 1,
        run: Some(run()),
        stage: "build".to_owned(),
        platform: "linux-x86_64".to_owned(),
        revision: REVISION.to_owned(),
        verdict: Verdict::Succeeded.into(),
        exit_code: 0,
        retry: Retry::Safe.into(),
        set_hash: SET_HASH.to_owned(),
        started_at_ms: 10,
        finished_at_ms: 20,
        actions: vec![action("compile")],
        ..StageReport::default()
    }
}

/// A stage report that ended in `verdict` with `reason`, its exit code and advice set from the
/// class the library gives, with `landed` says whether it lists a published coordinate.
fn ended(verdict: Verdict, reason: Reason, landed: bool) -> StageReport {
    let class = exit_class(verdict, reason, landed);
    let mut report = stage();
    report.verdict = verdict.into();
    report.reason = reason.into();
    report.exit_code = u32::from(class.code());
    report.retry = default_retry(class).into();
    if landed {
        report
            .actions
            .push(with_effect("publish", "conda/pkg", EffectState::Published));
    }
    report
}

fn with_effect(name: &str, subject: &str, state: EffectState) -> ActionReport {
    ActionReport {
        effects: vec![effect(subject, state)],
        ..action(name)
    }
}

fn refused(report: &StageReport) -> E {
    validate_stage(report).expect_err("the report is refused")
}

#[test]
fn a_report_that_ended_well_is_accepted() {
    assert_eq!(validate_stage(&stage()), Ok(()));
}

#[test]
fn a_report_the_class_table_describes_is_accepted_for_every_verdict_and_reason() {
    let verdicts: Vec<Verdict> = (0..64).filter_map(|n| Verdict::try_from(n).ok()).collect();
    let reasons: Vec<Reason> = (0..256).filter_map(|n| Reason::try_from(n).ok()).collect();
    for verdict in verdicts {
        if verdict == Verdict::Unspecified {
            continue;
        }
        for &reason in &reasons {
            // A reason is required with some verdicts and refused with others; the rows that
            // satisfy both are the ones checked here.
            let reason = match verdict {
                Verdict::Succeeded | Verdict::Noop => Reason::Unspecified,
                Verdict::Refused
                | Verdict::Failed
                | Verdict::Indeterminate
                | Verdict::ValidationFailed
                    if reason == Reason::Unspecified =>
                {
                    continue;
                }
                _ => reason,
            };
            for landed in [false, true] {
                assert_eq!(
                    validate_stage(&ended(verdict, reason, landed)),
                    Ok(()),
                    "{verdict:?} {reason:?} landed={landed}"
                );
            }
        }
    }
}

#[test]
fn the_schema_version_goes_through_the_policy() {
    for version in [-1, 0, 2, 3, 100] {
        let mut report = stage();
        report.schema_version = version;
        let want = report_schema().check(version).expect_err("refused");
        assert_eq!(refused(&report), E::Schema(want), "{version}");
        let run = RunReport {
            schema_version: version,
            ..a_run_that_ended_well()
        };
        assert_eq!(validate_run(&run), Err(E::Schema(want)), "{version}");
    }
    assert_eq!(report_schema().current(), 1);
}

#[test]
fn the_run_identity_is_required_and_checked() {
    let mut report = stage();
    report.run = None;
    assert_eq!(refused(&report), E::RunMissing);
    report.run = Some(RunIdentity {
        attempt: 0,
        ..run()
    });
    assert_eq!(refused(&report), E::Run(RunIdentityError::AttemptMissing));
    report.run = Some(RunIdentity {
        run_id: "Bad Id".to_owned(),
        ..run()
    });
    assert_eq!(
        refused(&report),
        E::Run(RunIdentityError::RunIdByte { at: 0 })
    );
}

#[test]
fn the_stage_needs_a_name() {
    let mut report = stage();
    report.stage = String::new();
    assert_eq!(refused(&report), E::StageNameEmpty);
}

#[test]
fn the_revision_is_empty_or_a_full_lowercase_commit_id() {
    let mut report = stage();
    report.revision = String::new();
    assert_eq!(validate_stage(&report), Ok(()));
    for text in [
        "0123456789ABCDEF0123456789abcdef01234567",
        "0123456789abcdef0123456789abcdef0123456",
        "0123456789abcdef0123456789abcdef012345678",
        "g123456789abcdef0123456789abcdef01234567",
        "main",
    ] {
        report.revision = text.to_owned();
        assert_eq!(
            refused(&report),
            E::Revision {
                text: text.to_owned()
            }
        );
    }
}

#[test]
fn the_set_hash_is_empty_or_sha256_in_lowercase_hex() {
    let mut report = stage();
    report.set_hash = String::new();
    assert_eq!(validate_stage(&report), Ok(()));
    let cases = vec![
        SET_HASH[1..].to_owned(),
        SET_HASH.to_uppercase(),
        format!("{SET_HASH}0"),
        format!("g{}", &SET_HASH[1..]),
    ];
    for text in cases {
        report.set_hash = text.clone();
        assert_eq!(refused(&report), E::SetHash { text });
    }
}

#[test]
fn the_selectors_are_well_formed_and_listed_once() {
    let mut report = stage();
    report.only = vec!["action:compile".to_owned(), "validation:smoke".to_owned()];
    assert_eq!(validate_stage(&report), Ok(()));
    for text in ["compile", "action:", "step:compile", ""] {
        report.only = vec![text.to_owned()];
        assert_eq!(
            refused(&report),
            E::OnlyMalformed {
                text: text.to_owned()
            }
        );
    }
    report.only = vec![
        "action:a".to_owned(),
        "validation:b".to_owned(),
        "action:a".to_owned(),
    ];
    assert_eq!(
        refused(&report),
        E::OnlyTwice {
            text: "action:a".to_owned()
        }
    );
}

#[test]
fn an_unset_verdict_is_refused() {
    let mut report = stage();
    report.verdict = Verdict::Unspecified.into();
    assert_eq!(refused(&report), E::VerdictUnspecified);
    report.verdict = 99;
    assert_eq!(refused(&report), E::VerdictUnspecified);
}

#[test]
fn a_verdict_that_stopped_the_run_names_its_reason() {
    for verdict in [
        Verdict::Refused,
        Verdict::Failed,
        Verdict::Indeterminate,
        Verdict::ValidationFailed,
    ] {
        let mut report = ended(verdict, Reason::BuildFailed, false);
        assert_eq!(validate_stage(&report), Ok(()), "{verdict:?}");
        report.reason = Reason::Unspecified.into();
        assert_eq!(refused(&report), E::ReasonRequired { verdict });
    }
    for verdict in [Verdict::Partial, Verdict::Interrupted, Verdict::Superseded] {
        let mut report = ended(verdict, Reason::Unspecified, false);
        assert_eq!(validate_stage(&report), Ok(()), "{verdict:?}");
        report.reason = Reason::SupersededByNewer.into();
        assert_eq!(validate_stage(&report), Ok(()), "{verdict:?}");
    }
}

#[test]
fn a_verdict_that_ended_well_carries_no_reason() {
    for verdict in [Verdict::Succeeded, Verdict::Noop] {
        let mut report = ended(verdict, Reason::Unspecified, false);
        report.reason = Reason::BuildFailed.into();
        assert_eq!(
            refused(&report),
            E::ReasonOnSuccess {
                verdict,
                reason: Reason::BuildFailed
            }
        );
    }
}

#[test]
fn the_exit_code_is_the_one_the_class_gives() {
    let mut report = ended(Verdict::Refused, Reason::StageUnknown, false);
    assert_eq!(report.exit_code, 3);
    report.exit_code = 4;
    assert_eq!(
        refused(&report),
        E::ExitCode {
            stated: 4,
            expected: 3
        }
    );
    // Something landed: a refusal after an effect is class 6, never 3.
    let report = StageReport {
        exit_code: 3,
        ..ended(Verdict::Refused, Reason::StageUnknown, true)
    };
    assert!(matches!(
        refused(&report),
        E::PromisesNoEffect { exit_code: 3, .. }
    ));
}

#[test]
fn a_lost_lease_is_class_six_whatever_the_verdict_says() {
    for reason in [Reason::LeaseLost, Reason::LeaseUnconfirmed] {
        let mut report = ended(Verdict::Refused, reason, false);
        assert_eq!(report.exit_code, 6);
        assert_eq!(validate_stage(&report), Ok(()));
        report.exit_code = 3;
        report.retry = Retry::NeedsHuman.into();
        assert_eq!(
            refused(&report),
            E::ExitCode {
                stated: 3,
                expected: 6
            }
        );
    }
}

#[test]
fn a_class_that_promises_no_effect_never_sits_on_a_report_that_lists_one() {
    for (verdict, code) in [(Verdict::Refused, 3), (Verdict::Failed, 4)] {
        for state in [
            EffectState::Published,
            EffectState::Staged,
            EffectState::Yanked,
        ] {
            let mut report = ended(verdict, Reason::PublishUpload, false);
            report.exit_code = code;
            report.actions = vec![with_effect("publish", "conda/pkg", state)];
            assert_eq!(
                refused(&report),
                E::PromisesNoEffect {
                    exit_code: code,
                    effect: "conda/pkg".to_owned()
                },
                "{verdict:?} {state:?}"
            );
        }
    }
    // Usage (2) is the same promise.
    let mut report = ended(Verdict::Failed, Reason::BuildFailed, true);
    report.exit_code = 2;
    assert!(matches!(
        refused(&report),
        E::PromisesNoEffect { exit_code: 2, .. }
    ));
}

#[test]
fn only_published_staged_and_yanked_effects_count_as_landed() {
    for state in [
        EffectState::Built,
        EffectState::AlreadyPresent,
        EffectState::NotReached,
        EffectState::Tested,
    ] {
        let mut report = ended(Verdict::Failed, Reason::PublishUpload, false);
        assert_eq!(report.exit_code, 4);
        report.actions = vec![with_effect("compile", "pkg", state)];
        assert_eq!(validate_stage(&report), Ok(()), "{state:?}");
    }
}

#[test]
fn the_retry_advice_is_set_and_never_weaker_than_the_class() {
    let mut report = stage();
    report.retry = Retry::Unspecified.into();
    assert_eq!(refused(&report), E::RetryUnspecified);
    // (class, accepted advice)
    let table: [(ExitClass, &[Retry]); 6] = [
        (
            ExitClass::Done,
            &[Retry::Safe, Retry::Unsafe, Retry::NeedsHuman],
        ),
        (
            ExitClass::FailedNoEffect,
            &[Retry::Safe, Retry::Unsafe, Retry::NeedsHuman],
        ),
        (
            ExitClass::FailedChanged,
            &[Retry::Unsafe, Retry::NeedsHuman],
        ),
        (ExitClass::Refused, &[Retry::NeedsHuman]),
        (ExitClass::Indeterminate, &[Retry::NeedsHuman]),
        (ExitClass::ValidationFailed, &[Retry::NeedsHuman]),
    ];
    let cases = [
        (Verdict::Succeeded, false),
        (Verdict::Failed, false),
        (Verdict::Partial, false),
        (Verdict::Refused, false),
        (Verdict::Indeterminate, false),
        (Verdict::ValidationFailed, false),
    ];
    for ((class, accepted), (verdict, landed)) in table.iter().zip(cases) {
        let base = ended(verdict, Reason::BuildFailed, landed);
        assert_eq!(exit_class(verdict, Reason::BuildFailed, landed), *class);
        for retry in [Retry::Safe, Retry::Unsafe, Retry::NeedsHuman] {
            let mut report = base.clone();
            report.retry = retry.into();
            if verdict == Verdict::Succeeded {
                report.reason = Reason::Unspecified.into();
            }
            if accepted.contains(&retry) {
                assert_eq!(validate_stage(&report), Ok(()), "{class:?} {retry:?}");
            } else {
                assert_eq!(
                    refused(&report),
                    E::RetryWeaker {
                        stated: retry,
                        default: default_retry(*class)
                    },
                    "{class:?} {retry:?}"
                );
            }
        }
    }
}

#[test]
fn the_default_advice_of_each_class() {
    assert_eq!(default_retry(ExitClass::Done), Retry::Safe);
    assert_eq!(default_retry(ExitClass::FailedNoEffect), Retry::Safe);
    assert_eq!(default_retry(ExitClass::FailedChanged), Retry::Unsafe);
    for class in [
        ExitClass::Usage,
        ExitClass::Refused,
        ExitClass::Indeterminate,
        ExitClass::ValidationFailed,
        ExitClass::Leaks,
    ] {
        assert_eq!(default_retry(class), Retry::NeedsHuman, "{class:?}");
    }
}

#[test]
fn a_stage_cannot_finish_before_it_started() {
    let mut report = stage();
    report.started_at_ms = 20;
    report.finished_at_ms = 20;
    assert_eq!(validate_stage(&report), Ok(()));
    report.finished_at_ms = 19;
    assert_eq!(
        refused(&report),
        E::FinishedBeforeStarted {
            stage: "build".to_owned(),
            started: 20,
            finished: 19
        }
    );
}

#[test]
fn a_time_of_zero_means_unknown_and_is_not_compared() {
    let mut report = stage();
    report.started_at_ms = 20;
    report.finished_at_ms = 0;
    assert_eq!(validate_stage(&report), Ok(()));
    report.started_at_ms = 0;
    report.finished_at_ms = 5;
    assert_eq!(validate_stage(&report), Ok(()));
}

#[test]
fn stages_of_a_run_are_not_ordered_by_time_because_hosts_differ() {
    let mut report = a_run_that_ended_well();
    report.stages[0].started_at_ms = 1_000;
    report.stages[0].finished_at_ms = 2_000;
    report.stages[1].started_at_ms = 10;
    report.stages[1].finished_at_ms = 20;
    assert_eq!(validate_run(&report), Ok(()));
}

#[test]
fn a_run_report_names_the_stage_that_finished_before_it_started() {
    let mut report = a_run_that_ended_well();
    report.stages[1].started_at_ms = 9;
    report.stages[1].finished_at_ms = 8;
    assert_eq!(
        refused_run(&report),
        E::Stage {
            index: 1,
            error: Box::new(E::FinishedBeforeStarted {
                stage: "publish".to_owned(),
                started: 9,
                finished: 8
            })
        }
    );
}

#[test]
fn every_action_has_its_own_name_and_a_verdict() {
    let mut report = stage();
    report.actions = vec![action("compile"), action("")];
    assert_eq!(refused(&report), E::ActionNameEmpty { index: 1 });
    report.actions = vec![action("compile"), action("test"), action("compile")];
    assert_eq!(
        refused(&report),
        E::ActionTwice {
            name: "compile".to_owned()
        }
    );
    let mut unset = action("compile");
    unset.verdict = Verdict::Unspecified.into();
    report.actions = vec![unset];
    assert_eq!(
        refused(&report),
        E::ActionVerdictUnspecified {
            name: "compile".to_owned()
        }
    );
}

#[test]
fn an_effect_with_no_state_is_refused() {
    let mut report = stage();
    let mut bad = with_effect("publish", "conda/pkg", EffectState::Published);
    bad.effects[0].state = EffectState::EffectUnspecified.into();
    report.actions = vec![bad];
    assert_eq!(
        refused(&report),
        E::EffectUnspecified {
            action: "publish".to_owned(),
            subject: "conda/pkg".to_owned()
        }
    );
}

#[test]
fn a_dry_run_lists_only_what_would_happen() {
    let happened = [
        EffectState::Built,
        EffectState::Published,
        EffectState::Staged,
        EffectState::Yanked,
        EffectState::Tested,
    ];
    let would = [
        EffectState::WouldBuild,
        EffectState::WouldPublish,
        EffectState::WouldTest,
    ];
    let mut report = ended(Verdict::Succeeded, Reason::Unspecified, false);
    report.plan = true;
    for state in happened {
        report.actions = vec![with_effect("a", "x", state)];
        assert_eq!(
            refused(&report),
            E::EffectInPlan {
                action: "a".to_owned(),
                subject: "x".to_owned()
            },
            "{state:?}"
        );
    }
    for state in would
        .into_iter()
        .chain([EffectState::AlreadyPresent, EffectState::NotReached])
    {
        report.actions = vec![with_effect("a", "x", state)];
        assert_eq!(validate_stage(&report), Ok(()), "{state:?}");
    }
    // A real run never lists what would happen.
    report.plan = false;
    for state in would {
        report.actions = vec![with_effect("a", "x", state)];
        assert_eq!(
            refused(&report),
            E::WouldOutsidePlan {
                action: "a".to_owned(),
                subject: "x".to_owned()
            },
            "{state:?}"
        );
    }
}

#[test]
fn a_row_with_no_state_never_reads_as_passed() {
    let row = |state: RowState| Row {
        name: "//pkg:test".to_owned(),
        state: state.into(),
        detail: String::new(),
    };
    let mut report = stage();
    let mut test = action("test");
    test.rows = vec![row(RowState::RowPassed), row(RowState::RowUnspecified)];
    report.actions = vec![test];
    let want = E::RowUnspecified {
        action: "test".to_owned(),
        row: "//pkg:test".to_owned(),
    };
    assert_eq!(refused(&report), want);
    let mut check = action("test");
    check.validations = vec![ValidationReport {
        name: "smoke".to_owned(),
        verdict: Verdict::Succeeded.into(),
        rows: vec![row(RowState::RowUnspecified)],
    }];
    report.actions = vec![check];
    assert_eq!(refused(&report), want);
    for state in [
        RowState::RowPassed,
        RowState::RowFailed,
        RowState::RowNotRun,
        RowState::RowNotReached,
        RowState::RowUnreadable,
    ] {
        let mut test = action("test");
        test.rows = vec![row(state)];
        report.actions = vec![test];
        assert_eq!(validate_stage(&report), Ok(()), "{state:?}");
    }
}

// ---- run reports ----

fn a_run_that_ended_well() -> RunReport {
    let mut second = stage();
    second.stage = "publish".to_owned();
    RunReport {
        schema_version: 1,
        run: Some(run()),
        stages: vec![stage(), second],
        verdict: Verdict::Succeeded.into(),
        reason: Reason::Unspecified.into(),
        exit_code: 0,
        retry: Retry::Safe.into(),
    }
}

/// A run that stopped at its second stage with `verdict` and `reason`.
fn a_run_that_stopped(verdict: Verdict, reason: Reason, landed_first: bool) -> RunReport {
    let mut run_report = a_run_that_ended_well();
    run_report.stages[1] = StageReport {
        stage: "publish".to_owned(),
        ..ended(verdict, reason, false)
    };
    if landed_first {
        run_report.stages[0]
            .actions
            .push(with_effect("push", "conda/pkg", EffectState::Published));
    }
    let class = exit_class(verdict, reason, landed_first);
    run_report.verdict = verdict.into();
    run_report.reason = reason.into();
    run_report.exit_code = u32::from(class.code());
    run_report.retry = default_retry(class).into();
    run_report
}

fn refused_run(report: &RunReport) -> E {
    validate_run(report).expect_err("the run report is refused")
}

#[test]
fn a_run_that_ended_well_is_accepted() {
    assert_eq!(validate_run(&a_run_that_ended_well()), Ok(()));
}

#[test]
fn a_run_that_stopped_reports_the_stage_that_stopped_it() {
    let report = a_run_that_stopped(Verdict::Failed, Reason::PublishUpload, false);
    assert_eq!(report.exit_code, 4);
    assert_eq!(validate_run(&report), Ok(()));
}

#[test]
fn a_run_needs_a_run_identity_and_a_stage() {
    let mut report = a_run_that_ended_well();
    report.run = None;
    assert_eq!(refused_run(&report), E::RunMissing);
    report.run = Some(RunIdentity {
        run_id: String::new(),
        ..run()
    });
    assert_eq!(refused_run(&report), E::Run(RunIdentityError::RunIdEmpty));
    let mut report = a_run_that_ended_well();
    report.stages.clear();
    assert_eq!(refused_run(&report), E::NoStages);
}

#[test]
fn a_stage_of_another_run_is_refused() {
    for other in [
        RunIdentity {
            run_id: "gh-43".to_owned(),
            ..run()
        },
        RunIdentity {
            attempt: 2,
            ..run()
        },
        RunIdentity {
            context: [].into(),
            ..run()
        },
    ] {
        let mut report = a_run_that_ended_well();
        report.stages[1].run = Some(other.clone());
        assert_eq!(
            refused_run(&report),
            E::StageRunDiffers { index: 1 },
            "{other:?}"
        );
    }
}

#[test]
fn a_stage_name_is_used_once_in_a_run() {
    let mut report = a_run_that_ended_well();
    report.stages[1].stage = "build".to_owned();
    assert_eq!(
        refused_run(&report),
        E::StageTwice {
            name: "build".to_owned()
        }
    );
}

#[test]
fn a_refused_stage_report_refuses_the_run_and_says_which() {
    let mut report = a_run_that_ended_well();
    report.stages[1].revision = "main".to_owned();
    assert_eq!(
        refused_run(&report),
        E::Stage {
            index: 1,
            error: Box::new(E::Revision {
                text: "main".to_owned()
            })
        }
    );
    // The stage's own run is checked as a stage is: a missing one is refused as missing.
    let mut report = a_run_that_ended_well();
    report.stages[0].run = None;
    assert_eq!(
        refused_run(&report),
        E::Stage {
            index: 0,
            error: Box::new(E::RunMissing)
        }
    );
}

#[test]
fn the_run_takes_the_verdict_and_the_reason_of_the_first_stage_that_did_not_end_done() {
    let mut report = a_run_that_stopped(Verdict::Failed, Reason::PublishUpload, false);
    report.verdict = Verdict::Succeeded.into();
    assert_eq!(
        refused_run(&report),
        E::RunVerdictDiffers {
            stated: Verdict::Succeeded,
            expected: Verdict::Failed
        }
    );
    let mut report = a_run_that_stopped(Verdict::Failed, Reason::PublishUpload, false);
    report.reason = Reason::Credential.into();
    assert_eq!(
        refused_run(&report),
        E::RunReasonDiffers {
            stated: Reason::Credential,
            expected: Reason::PublishUpload
        }
    );
}

#[test]
fn a_run_whose_stages_all_ended_done_ends_done() {
    for verdict in [Verdict::Failed, Verdict::Partial, Verdict::Unspecified] {
        let mut report = a_run_that_ended_well();
        report.verdict = verdict.into();
        assert_eq!(refused_run(&report), E::RunNotDone { stated: verdict });
    }
    let mut report = a_run_that_ended_well();
    report.stages[0].verdict = Verdict::Noop.into();
    report.verdict = Verdict::Noop.into();
    assert_eq!(validate_run(&report), Ok(()));
}

#[test]
fn the_run_exit_code_counts_what_any_stage_landed() {
    // The first stage published; the second failed with nothing of its own landed. The stage
    // alone is class 4; the run is class 6.
    let report = a_run_that_stopped(Verdict::Failed, Reason::PublishUpload, true);
    assert_eq!(report.exit_code, 6);
    assert_eq!(report.stages[1].exit_code, 4);
    assert_eq!(validate_run(&report), Ok(()));
    let mut wrong = report.clone();
    wrong.exit_code = 4;
    wrong.retry = Retry::Safe.into();
    assert!(matches!(
        refused_run(&wrong),
        E::PromisesNoEffect { exit_code: 4, .. }
    ));
    let mut wrong = report;
    wrong.exit_code = 5;
    assert_eq!(
        refused_run(&wrong),
        E::ExitCode {
            stated: 5,
            expected: 6
        }
    );
}

#[test]
fn the_run_retry_advice_follows_the_run_class() {
    let mut report = a_run_that_stopped(Verdict::Failed, Reason::PublishUpload, true);
    report.retry = Retry::Safe.into();
    assert_eq!(
        refused_run(&report),
        E::RetryWeaker {
            stated: Retry::Safe,
            default: Retry::Unsafe
        }
    );
    report.retry = Retry::Unspecified.into();
    assert_eq!(refused_run(&report), E::RetryUnspecified);
}

#[test]
fn a_reason_number_this_reader_does_not_know_is_refused() {
    let mut report = stage();
    report.reason = 9_999;
    assert_eq!(refused(&report), E::ReasonUnknown { number: 9_999 });
}

#[test]
fn every_refusal_reads_as_a_sentence_that_names_what_it_refused() {
    let k = |s: &str| s.to_owned();
    let stage_refusal = E::Stage {
        index: 2,
        error: Box::new(E::StageNameEmpty),
    };
    let cases = [
        (
            E::Schema(report_schema().check(0).expect_err("refused")),
            "schema_version",
        ),
        (E::RunMissing, "no run"),
        (E::Run(RunIdentityError::RunIdEmpty), "run id is empty"),
        (E::StageNameEmpty, "no name"),
        (E::Revision { text: k("main") }, "'main'"),
        (E::OnlyMalformed { text: k("x") }, "'x'"),
        (
            E::OnlyTwice {
                text: k("action:a"),
            },
            "'action:a'",
        ),
        (E::SetHash { text: k("ab") }, "'ab'"),
        (E::VerdictUnspecified, "verdict"),
        (E::ReasonUnknown { number: 77 }, "77"),
        (
            E::ReasonRequired {
                verdict: Verdict::Failed,
            },
            "Failed",
        ),
        (
            E::ReasonOnSuccess {
                verdict: Verdict::Noop,
                reason: Reason::Channel,
            },
            "Channel",
        ),
        (
            E::ExitCode {
                stated: 3,
                expected: 6,
            },
            "exit code 3 is not 6",
        ),
        (
            E::PromisesNoEffect {
                exit_code: 4,
                effect: k("conda/pkg"),
            },
            "'conda/pkg'",
        ),
        (E::RetryUnspecified, "retry"),
        (
            E::RetryWeaker {
                stated: Retry::Safe,
                default: Retry::Unsafe,
            },
            "Unsafe",
        ),
        (
            E::FinishedBeforeStarted {
                stage: k("pub"),
                started: 9,
                finished: 8,
            },
            "stage 'pub' finished at 8 ms",
        ),
        (E::StageAfterStop { index: 5 }, "after stage report 5"),
        (E::PlanDiffers { index: 6 }, "stage report 6"),
        (E::ActionNameEmpty { index: 4 }, "action 4"),
        (E::ActionTwice { name: k("dup") }, "'dup'"),
        (E::ActionVerdictUnspecified { name: k("a1") }, "'a1'"),
        (
            E::EffectUnspecified {
                action: k("a2"),
                subject: k("s2"),
            },
            "'s2'",
        ),
        (
            E::EffectInPlan {
                action: k("a3"),
                subject: k("s3"),
            },
            "dry run",
        ),
        (
            E::WouldOutsidePlan {
                action: k("a4"),
                subject: k("s4"),
            },
            "real run",
        ),
        (
            E::RowUnspecified {
                action: k("a5"),
                row: k("r5"),
            },
            "'r5'",
        ),
        (E::NoStages, "no stage"),
        (E::StageRunDiffers { index: 3 }, "another run"),
        (E::StageTwice { name: k("build") }, "'build'"),
        (stage_refusal, "stage report 2: the stage has no name"),
        (
            E::RunVerdictDiffers {
                stated: Verdict::Succeeded,
                expected: Verdict::Failed,
            },
            "Failed",
        ),
        (
            E::RunReasonDiffers {
                stated: Reason::Channel,
                expected: Reason::Credential,
            },
            "Credential",
        ),
        (
            E::RunNotDone {
                stated: Verdict::Failed,
            },
            "every stage ended done",
        ),
    ];
    for (error, want) in cases {
        let text = error.to_string();
        assert!(text.contains(want), "{text:?} lacks {want:?}");
        assert!(!text.contains('\n'), "{text:?}");
    }
    let boxed: Box<dyn std::error::Error> = Box::new(E::NoStages);
    assert_eq!(boxed.to_string(), E::NoStages.to_string());
}

#[test]
fn a_stage_that_did_not_end_done_is_the_last_stage_of_the_run() {
    // A refusal that promises no change, then a later stage that could not tell: the run must
    // not read as the refusal.
    let mut report = a_run_that_ended_well();
    report.stages[0] = ended(Verdict::Refused, Reason::StageUnknown, false);
    report.stages[1] = StageReport {
        stage: "publish".to_owned(),
        ..ended(Verdict::Indeterminate, Reason::CannotTell, false)
    };
    report.verdict = Verdict::Refused.into();
    report.reason = Reason::StageUnknown.into();
    report.exit_code = 3;
    report.retry = Retry::NeedsHuman.into();
    assert_eq!(refused_run(&report), E::StageAfterStop { index: 0 });
    // Even a stage that ended done does not follow a stop.
    report.stages[1] = StageReport {
        stage: "publish".to_owned(),
        ..stage()
    };
    assert_eq!(refused_run(&report), E::StageAfterStop { index: 0 });
    // The stop is the last stage: accepted.
    report.stages.swap(0, 1);
    report.stages[0].stage = "build".to_owned();
    report.stages[1].stage = "publish".to_owned();
    assert_eq!(validate_run(&report), Ok(()));
}

#[test]
fn every_stage_of_a_run_is_a_dry_run_or_none_is() {
    let mut report = a_run_that_ended_well();
    report.stages[1].plan = true;
    assert_eq!(refused_run(&report), E::PlanDiffers { index: 1 });
    report.stages[0].plan = true;
    assert_eq!(validate_run(&report), Ok(()));
    report.stages[0].plan = false;
    assert_eq!(refused_run(&report), E::PlanDiffers { index: 1 });
}
