// SPDX-License-Identifier: Apache-2.0
//! What the fake issuer writes into a grant and the fake registry reads back: the scope name,
//! the repository and the coordinates it covers. A grant's secret is only ever these bytes.

use ucdk_release_proto::v1::Coordinate;

/// The text a coordinate is kept under.
pub fn coordinate_key(c: &Coordinate) -> String {
    format!(
        "{}/{}/{}/{}/{}",
        c.channel, c.r#type, c.name, c.version, c.platform
    )
}

/// What a grant says it covers.
pub struct Claim {
    pub scope: String,
    pub repository: String,
    pub coordinates: Vec<String>,
}

pub fn encode(scope: &str, repository: &str, coordinates: &[Coordinate]) -> Vec<u8> {
    let mut lines = vec![scope.to_owned(), repository.to_owned()];
    lines.extend(coordinates.iter().map(coordinate_key));
    lines.join("\n").into_bytes()
}

/// Reads what [`encode`] wrote; bytes that are not that read as a claim that covers nothing.
pub fn decode(secret: &[u8]) -> Claim {
    let text = String::from_utf8_lossy(secret);
    let mut lines = text.lines().map(str::to_owned);
    Claim {
        scope: lines.next().unwrap_or_default(),
        repository: lines.next().unwrap_or_default(),
        coordinates: lines.collect(),
    }
}
