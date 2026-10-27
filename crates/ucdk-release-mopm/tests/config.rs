// SPDX-License-Identifier: Apache-2.0
//! The configuration: the construction-time budget check, the locations it names and the trust
//! bundle.

mod common;

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process;
use std::time::Duration;

use common::{config, secs};
use ucdk_release_mopm::{
    Call, Config, ConfigError, MAX_PAGE, MOPM_TOKEN_TTL, Origin, Proxy, Roots, RootsError, Url,
    UrlError,
};

/// A file in the temporary directory that is removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str, contents: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!("ucdk-mopm-{}-{name}", process::id()));
        fs::write(&path, contents).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

const CERT: &str = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";

#[test]
fn the_proposed_defaults_fit_with_room_to_spare() {
    let config = config();
    assert_eq!(config.budget(Call::Stage), secs(300));
    assert_eq!(config.budget(Call::Commit), secs(450));
    assert_eq!(config.budget(Call::Abort), secs(15));
    assert_eq!(config.bound(), Ok(secs(595)));
    assert_eq!(config.check(), Ok(()));
}

#[test]
fn a_budget_at_the_bound_is_accepted_and_one_second_over_is_not() {
    // bound = min(600, 600 - 5) = 595; a commit is 450 + 60 of margin, so 85 seconds are free.
    let mut at_bound = config();
    at_bound.poll_deadline += secs(85);
    assert_eq!(at_bound.budget(Call::Commit) + at_bound.margin, secs(595));
    assert_eq!(at_bound.check(), Ok(()));

    let mut over = config();
    over.poll_deadline += secs(86);
    assert_eq!(
        over.check(),
        Err(ConfigError::Budget {
            call: Call::Commit,
            needed: secs(596),
            bound: secs(595)
        })
    );
}

#[test]
fn every_term_of_the_budget_counts_in_the_call_it_belongs_to() {
    type Edit = fn(&mut Config);
    // Each edit takes its call exactly one second over the bound of 595 with the 60 of margin.
    let edits: [(Call, Edit); 5] = [
        (Call::Stage, |c| c.upload_budget += secs(236)),
        (Call::Commit, |c| c.commit_request_timeout += secs(86)),
        (Call::Commit, |c| c.poll_deadline += secs(86)),
        (Call::Abort, |c| c.abort_timeout += secs(521)),
        (Call::Commit, |c| c.margin += secs(86)),
    ];
    for (call, edit) in edits {
        let mut config = config();
        edit(&mut config);
        assert_eq!(
            config.check(),
            Err(ConfigError::Budget {
                call,
                needed: secs(596),
                bound: secs(595)
            })
        );
    }
}

#[test]
fn the_calls_are_bounded_one_at_a_time_and_not_by_their_sum() {
    // Stage, commit and abort are separate seam calls, each under a fresh grant and a renewed
    // lease, so a configuration whose three budgets together are far over the bound is fine
    // when each one fits.
    let config = config();
    let sum = Call::ALL
        .into_iter()
        .map(|call| config.budget(call))
        .sum::<Duration>();
    assert_eq!(sum, secs(765));
    assert!(sum + config.margin > config.bound().unwrap());
    assert_eq!(config.check(), Ok(()));
}

#[test]
fn the_token_life_bounds_a_call_when_the_lease_is_longer() {
    let mut config = config();
    config.lease_ttl = secs(3600);
    assert_eq!(config.bound(), Ok(MOPM_TOKEN_TTL));
    config.poll_deadline += secs(90);
    assert_eq!(config.budget(Call::Commit) + config.margin, secs(600));
    assert_eq!(config.check(), Ok(()));
    config.poll_deadline += secs(1);
    assert_eq!(
        config.check(),
        Err(ConfigError::Budget {
            call: Call::Commit,
            needed: secs(601),
            bound: secs(600)
        })
    );
}

#[test]
fn the_lease_less_the_skew_bounds_a_call_when_the_token_is_longer() {
    let mut config = config();
    config.lease_ttl = secs(300);
    assert_eq!(config.bound(), Ok(secs(295)));
    assert!(matches!(
        config.check(),
        Err(ConfigError::Budget {
            call: Call::Stage,
            ..
        })
    ));
    config.upload_budget = secs(215);
    config.poll_deadline = secs(80);
    config.commit_request_timeout = secs(20);
    config.abort_timeout = secs(15);
    config.margin = secs(80);
    assert_eq!(config.budget(Call::Stage) + config.margin, secs(295));
    assert_eq!(config.check(), Ok(()));
}

#[test]
fn a_lease_not_longer_than_the_skew_leaves_no_room() {
    for lease in [secs(0), secs(3), secs(5)] {
        let mut config = config();
        config.lease_ttl = lease;
        assert_eq!(
            config.bound(),
            Err(ConfigError::LeaseTooShort { lease_ttl: lease })
        );
        assert_eq!(
            config.check(),
            Err(ConfigError::LeaseTooShort { lease_ttl: lease })
        );
    }
    let mut six = config();
    six.lease_ttl = secs(6);
    assert!(matches!(six.check(), Err(ConfigError::Budget { .. })));
}

#[test]
fn an_absurd_budget_fails_the_check_and_does_not_wrap() {
    let mut config = config();
    config.poll_deadline = std::time::Duration::MAX;
    config.abort_timeout = std::time::Duration::MAX;
    assert!(matches!(config.check(), Err(ConfigError::Budget { .. })));
}

#[test]
fn a_zero_duration_that_cannot_be_zero_is_refused_by_name() {
    type Edit = fn(&mut Config);
    let edits: [(&str, Edit); 7] = [
        ("request_timeout", |c| c.request_timeout = secs(0)),
        ("connect_timeout", |c| c.connect_timeout = secs(0)),
        ("poll_interval", |c| c.poll_interval = secs(0)),
        ("upload_budget", |c| c.upload_budget = secs(0)),
        ("commit_request_timeout", |c| {
            c.commit_request_timeout = secs(0);
        }),
        // A zero abort budget leaves an abort no time at all, and a zero margin leaves no room
        // for the exchange-to-call gap and a connect timeout.
        ("abort_timeout", |c| c.abort_timeout = secs(0)),
        ("margin", |c| c.margin = secs(0)),
    ];
    for (name, edit) in edits {
        let mut config = config();
        edit(&mut config);
        assert_eq!(config.check(), Err(ConfigError::ZeroDuration(name)));
    }
    // These may be zero: a zero poll deadline returns as soon as the commit is accepted, and a
    // zero ceiling turns every `Retry-After` into a refusal instead of a wait.
    let mut config = config();
    config.poll_deadline = secs(0);
    config.retry_after_ceiling = secs(0);
    assert_eq!(config.check(), Ok(()));
}

#[test]
fn the_page_size_is_one_mopm_answers() {
    for page in [0, MAX_PAGE + 1, u32::MAX] {
        let mut config = config();
        config.newest_page = page;
        assert_eq!(config.check(), Err(ConfigError::Page(page)));
    }
    for page in [1, 50, MAX_PAGE] {
        let mut config = config();
        config.newest_page = page;
        assert_eq!(config.check(), Ok(()));
    }
}

#[test]
fn a_registry_location_with_a_query_is_refused() {
    let mut config = config();
    config.registry = Url::parse("https://registry.example.com/api/v1?key=1").unwrap();
    assert_eq!(config.check(), Err(ConfigError::RegistryQuery));
}

#[test]
fn a_file_url_may_name_only_the_registry_or_a_listed_file_host() {
    let mut config = config();
    config.file_hosts = vec![Origin::parse("https://files.example.com").unwrap()];
    let allowed = [
        "https://registry.example.com/files/a.conda",
        "https://registry.example.com:443/files/a.conda",
        "https://REGISTRY.example.com/x",
        "https://files.example.com/store/a?X-Amz-Signature=abc",
    ];
    for url in allowed {
        assert!(
            config.allows_file_origin(&Url::parse(url).unwrap()),
            "{url}"
        );
    }
    let refused = [
        "https://evil.example.com/a.conda",
        "https://registry.example.com:8443/a.conda",
        "https://registry.example.com.evil.example.net/a",
        "https://files.example.com:444/a",
        "https://sub.files.example.com/a",
    ];
    for url in refused {
        assert!(
            !config.allows_file_origin(&Url::parse(url).unwrap()),
            "{url}"
        );
    }
}

#[test]
fn locations_are_https_only_and_exact() {
    let url = Url::parse("https://Registry.Example.com/api/v1").unwrap();
    assert_eq!(url.origin().host(), "registry.example.com");
    assert_eq!(url.origin().port(), 443);
    assert_eq!(url.path(), "/api/v1");
    assert!(!url.has_query());
    assert_eq!(Url::parse("https://h.example.com").unwrap().path(), "/");
    assert_eq!(
        Url::parse("https://h.example.com:8443/x")
            .unwrap()
            .origin()
            .port(),
        8443
    );

    let cases = [
        ("http://registry.example.com", UrlError::NotHttps),
        ("registry.example.com", UrlError::NotHttps),
        ("ftp://registry.example.com", UrlError::NotHttps),
        ("https://", UrlError::Host),
        ("https://:443", UrlError::Host),
        ("https://user@registry.example.com", UrlError::Host),
        ("https://[::1]/", UrlError::Host),
        ("https://registry.example.com:0/", UrlError::Port),
        ("https://registry.example.com:+443/", UrlError::Port),
        ("https://registry.example.com:65536/", UrlError::Port),
        ("https://registry.example.com:/", UrlError::Port),
        ("https://registry.example.com:1:2/", UrlError::Port),
        ("https://registry.example.com/a#frag", UrlError::Fragment),
        ("https://registry.example.com/a b", UrlError::Character),
        ("https://registry.example.com/\u{e9}", UrlError::Character),
        ("https://registry.example.com/a\n", UrlError::Character),
    ];
    for (text, expected) in cases {
        assert_eq!(Url::parse(text).err(), Some(expected), "{text}");
    }
}

#[test]
fn an_origin_has_no_path_and_no_query() {
    let origin = Origin::parse("https://Files.Example.com").unwrap();
    assert_eq!(origin.to_string(), "https://files.example.com");
    assert_eq!(Origin::parse("https://files.example.com/").unwrap(), origin);
    assert_eq!(
        Origin::parse("https://files.example.com:443").unwrap(),
        origin
    );
    let other = Origin::parse("https://files.example.com:8443").unwrap();
    assert_eq!(other.to_string(), "https://files.example.com:8443");
    for text in ["https://files.example.com/x", "https://files.example.com?x"] {
        assert_eq!(Origin::parse(text), Err(UrlError::NotOrigin), "{text}");
    }
    assert_eq!(
        Origin::parse("http://files.example.com"),
        Err(UrlError::NotHttps)
    );
}

#[test]
fn a_proxy_is_an_explicit_host_and_port() {
    let proxy = Proxy::parse("Proxy.Example.com:3128").unwrap();
    assert_eq!(proxy.host(), "proxy.example.com");
    assert_eq!(proxy.port(), 3128);
    assert_eq!(Proxy::parse("proxy.example.com"), Err(UrlError::Port));
    assert_eq!(Proxy::parse("proxy.example.com:0"), Err(UrlError::Port));
    assert_eq!(Proxy::parse(":3128"), Err(UrlError::Host));
    assert_eq!(Proxy::parse("a b:3128"), Err(UrlError::Host));
}

#[test]
fn the_trust_bundle_is_the_named_file() {
    let file = Scratch::new("roots-ok.pem", CERT.as_bytes());
    let roots = Roots::Bundle(file.0.clone());
    assert_eq!(roots.bundle_pem().unwrap(), CERT);
    let two = Scratch::new("roots-two.pem", format!("{CERT}# note\n{CERT}").as_bytes());
    assert_eq!(
        Roots::Bundle(two.0.clone())
            .bundle_pem()
            .unwrap()
            .matches("BEGIN")
            .count(),
        2
    );
}

#[test]
fn a_trust_bundle_that_trusts_nothing_or_is_broken_is_refused() {
    let missing = std::env::temp_dir().join(format!("ucdk-mopm-{}-missing.pem", process::id()));
    assert_eq!(
        Roots::Bundle(missing).bundle_pem(),
        Err(RootsError::Unreadable(ErrorKind::NotFound))
    );
    let binary = Scratch::new("roots-binary.pem", &[0xff, 0xfe, 0x00]);
    assert_eq!(
        Roots::Bundle(binary.0.clone()).bundle_pem(),
        Err(RootsError::Unreadable(ErrorKind::InvalidData))
    );
    let empty = Scratch::new("roots-empty.pem", b"");
    assert_eq!(
        Roots::Bundle(empty.0.clone()).bundle_pem(),
        Err(RootsError::NoCertificate)
    );
    let key = Scratch::new(
        "roots-key.pem",
        b"-----BEGIN PRIVATE KEY-----\nAA\n-----END PRIVATE KEY-----\n",
    );
    assert_eq!(
        Roots::Bundle(key.0.clone()).bundle_pem(),
        Err(RootsError::NoCertificate)
    );
    let open = Scratch::new("roots-open.pem", b"-----BEGIN CERTIFICATE-----\nMIIB\n");
    assert_eq!(
        Roots::Bundle(open.0.clone()).bundle_pem(),
        Err(RootsError::Unterminated)
    );
    let half = Scratch::new(
        "roots-half.pem",
        format!("{CERT}-----BEGIN CERTIFICATE-----\nMIIB\n").as_bytes(),
    );
    assert_eq!(
        Roots::Bundle(half.0.clone()).bundle_pem(),
        Err(RootsError::Unterminated)
    );
}

#[test]
fn every_refusal_says_what_it_is() {
    let errors: Vec<Box<dyn std::error::Error>> = vec![
        Box::new(ConfigError::RegistryQuery),
        Box::new(ConfigError::ZeroDuration("poll_interval")),
        Box::new(ConfigError::Page(0)),
        Box::new(ConfigError::LeaseTooShort { lease_ttl: secs(3) }),
        Box::new(ConfigError::Budget {
            call: Call::Stage,
            needed: secs(596),
            bound: secs(595),
        }),
        Box::new(RootsError::Unreadable(ErrorKind::NotFound)),
        Box::new(RootsError::NoCertificate),
        Box::new(RootsError::Unterminated),
        Box::new(UrlError::NotHttps),
        Box::new(UrlError::Character),
        Box::new(UrlError::Host),
        Box::new(UrlError::Port),
        Box::new(UrlError::Fragment),
        Box::new(UrlError::NotOrigin),
    ];
    for error in errors {
        assert!(!error.to_string().is_empty());
    }
    for (call, name) in [
        (Call::Stage, "stage"),
        (Call::Commit, "commit"),
        (Call::Abort, "abort"),
    ] {
        assert_eq!(
            ConfigError::Budget {
                call,
                needed: secs(596),
                bound: secs(595)
            }
            .to_string(),
            format!(
                "a {name} call may take 596s with its margin, but the token and the lease allow 595s"
            )
        );
    }
    assert_eq!(
        ConfigError::ZeroDuration("poll_interval").to_string(),
        "poll_interval must be more than zero"
    );
}
