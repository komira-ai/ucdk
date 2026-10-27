// SPDX-License-Identifier: Apache-2.0
//! Building and sending a request, behind the grammar check: a coordinate mopm cannot hold never
//! produces a request, so nothing reaches the transport for it.

use ucdk_release_proto::v1::Coordinate;
use ucdk_release_seams::SeamError;

use crate::config::Config;
use crate::errors::{Op, map_transport};
use crate::mapping::check_key;
use crate::transport::{Header, Method, Request, Response, Transport};

/// An anonymous `GET` of `url`: reads carry no token.
pub(crate) fn get(url: crate::url::Url) -> Request<'static> {
    Request {
        method: Method::Get,
        url,
        headers: vec![Header::plain("accept", "application/json")],
        body: &[],
    }
}

/// `GET /packages/{name}/{version}` for `k`, the read `observe` is built on: public, with no
/// token. `Refused(Channel)` before anything is sent when `k` is not a coordinate of the
/// configured channel that mopm can hold; a transport failure is `Transient`. The answer is
/// returned whatever its status; classifying it is [`crate::map_reply`].
pub fn read_version<T: Transport>(
    transport: &mut T,
    config: &Config,
    k: &Coordinate,
) -> Result<Response, SeamError> {
    let key = check_key(&config.channel, k)?;
    let request = get(config.registry.join(&["packages", &key.name, &key.version]));
    transport
        .send(&request)
        .map_err(|e| map_transport(Op::Read, &e))
}
