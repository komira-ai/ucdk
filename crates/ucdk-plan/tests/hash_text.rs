// SPDX-License-Identifier: Apache-2.0
//! A plan hash read back from its text, as an approver pins it.

use ucdk_plan::PlanHash;

#[test]
fn plan_hash_reads_back_its_own_text() {
    let text = "0123456789abcdef".repeat(4);
    let hash = PlanHash::from_hex(&text).expect("a well-formed hash");
    assert_eq!(hash.as_str(), text);
    assert_eq!(hash.to_string(), text);
}

#[test]
fn plan_hash_text_that_is_not_64_lower_case_hex_digits_is_refused() {
    let good = "0123456789abcdef".repeat(4);
    let mut bad = vec![
        String::new(),
        good[1..].to_owned(),
        format!("{good}0"),
        good.to_uppercase(),
    ];
    // Each character just outside the digit and letter ranges, in the last place.
    for c in ['/', ':', '`', 'g', ' ', 'é'] {
        bad.push(format!("{}{c}", &good[..63]));
    }
    for text in bad {
        assert!(PlanHash::from_hex(&text).is_none(), "{text:?}");
    }
}
