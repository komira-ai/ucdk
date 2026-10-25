// SPDX-License-Identifier: Apache-2.0
//! The catalog response: a list that lowers is answered with its nodes in dependency order, and
//! one that does not with its findings.

mod common;

use common::{list_of, lowered, service_of};
use ucdk_api::catalog_response;
use ucdk_api::v1::ExitClass;
use ucdk_plan::lower;

/// Proves: a good list answers with one entry per lowered node, in the order the lowering gives,
/// with kind, wanted flag and dependencies, exit class done and no findings. Catches a response
/// that drops a node, reorders them, or reports a refusal for a good list.
#[test]
fn a_list_that_lowers_answers_with_its_nodes() {
    let list = list_of(vec![service_of("web", 80)]);
    let answer = catalog_response(Ok(lowered(&list)));
    let expected = lowered(&list);
    assert_eq!(answer.nodes.len(), expected.nodes().len());
    assert!(!answer.nodes.is_empty());
    for (got, want) in answer.nodes.iter().zip(expected.nodes()) {
        assert_eq!(got.path, want.node.path().to_string());
        assert_eq!(got.kind, want.node.kind().as_str());
        assert_eq!(got.wanted, want.wanted);
        assert_eq!(got.deps.len(), want.deps.len());
    }
    assert!(answer.findings.is_empty());
    assert_eq!(answer.exit, ExitClass::Done as i32);
}

/// Proves: a refused list answers with its findings, no nodes, and exit class refused. Catches
/// a refusal that still lists nodes or exits done.
#[test]
fn a_list_that_is_refused_answers_with_its_findings() {
    let list = list_of(vec![service_of("web", 80), service_of("web", 81)]);
    let findings = lower(&list).expect_err("a duplicate id is refused");
    let answer = catalog_response(Err(findings.clone()));
    assert_eq!(answer.findings.len(), findings.len());
    assert!(!answer.findings.is_empty());
    assert!(answer.nodes.is_empty());
    assert_eq!(answer.exit, ExitClass::Refused as i32);
}
