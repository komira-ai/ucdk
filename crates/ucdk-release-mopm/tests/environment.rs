// SPDX-License-Identifier: Apache-2.0
//! The trust anchor of a request that carries a bearer token does not depend on the
//! environment: the usual certificate and proxy variables, set to a bogus file, change nothing.
//!
//! The workspace forbids `unsafe`, so a test cannot set a variable in its own process. The
//! parent test starts this same test binary with the variables set and the child asserts.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use ucdk_release_mopm::Roots;

const BUNDLE: &str = "UCDK_MOPM_CHILD_BUNDLE";
const CERT: &str = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
const VARIABLES: [&str; 9] = [
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "REQUESTS_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "NO_PROXY",
];

/// Runs only in the child: with every variable pointing at something that does not exist, the
/// bundle is still exactly the named file.
#[test]
fn child_loads_the_named_bundle() {
    let Some(path) = std::env::var_os(BUNDLE) else {
        return;
    };
    for name in VARIABLES {
        let value = std::env::var(name).unwrap();
        assert!(value.contains("bogus"), "{name} was not set for the child");
    }
    let loaded = Roots::Bundle(PathBuf::from(path)).bundle_pem().unwrap();
    assert_eq!(loaded, CERT);
}

#[test]
fn certificate_variables_change_nothing() {
    let path = std::env::temp_dir().join(format!("ucdk-mopm-{}-env.pem", std::process::id()));
    fs::write(&path, CERT).unwrap();
    // Without the variables.
    assert_eq!(Roots::Bundle(path.clone()).bundle_pem().unwrap(), CERT);
    // With every one of them naming something that is not there.
    let exe = std::env::current_exe().unwrap();
    let mut child = Command::new(exe);
    child
        .args(["--exact", "child_loads_the_named_bundle", "--nocapture"])
        .env(BUNDLE, &path);
    for name in VARIABLES {
        child.env(name, "/nonexistent/bogus-certificates");
    }
    let output = child.output().unwrap();
    let _ = fs::remove_file(&path);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("1 passed"), "the child did not run: {text}");
}
