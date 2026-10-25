// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code, unsafe_code)]
//! Helpers shared by the ABI tests: the C functions called the way a C caller would, a resource
//! list, and a provider whose `observe` runs a hook.

use std::ptr::{self, NonNull};

use prost::Message as _;
use ucdk_api::v1::{Error, SessionConfig};
use ucdk_capi::{Buf, SessionHandle, ucdk_buf_free, ucdk_session_open};
use ucdk_proto::v1::resource::Body;
use ucdk_proto::v1::{Image, Resource, ResourceList, Service, image};
use ucdk_provider::{
    CellScope, Created, Deletability, Kind, Live, Node, Observation, OwnedObject, OwnerScope,
    PhysicalId, Provider, ProviderError, StampMode, StateClass, Updated, kit,
};
use ucdk_provider_fake::Fake;

/// The signature all five verbs share.
pub type VerbFn = unsafe extern "C" fn(*mut SessionHandle, *const u8, usize, *mut Buf) -> i32;

/// What a call answered: its status and the bytes in `out`, already freed.
#[derive(Debug)]
pub struct Reply {
    pub status: i32,
    pub bytes: Vec<u8>,
}

/// Copies the bytes of a buffer the library filled and frees it.
pub fn take(buf: Buf) -> Vec<u8> {
    if buf.ptr.is_null() {
        assert_eq!(buf.len, 0, "a buffer with no bytes has no length");
        return Vec::new();
    }
    // SAFETY: the library filled `buf` with `len` bytes at `ptr` and nothing has freed it.
    let bytes = unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) }.to_vec();
    // SAFETY: `buf` is as the library gave it out, freed here once.
    unsafe { ucdk_buf_free(buf) };
    bytes
}

/// A buffer that is not empty and is not the library's: a call must overwrite it, never free it.
pub fn junk() -> Buf {
    Buf {
        ptr: NonNull::<u8>::dangling().as_ptr(),
        len: 7,
    }
}

/// Calls a verb the way C would, with `out` starting as junk.
pub fn call(verb: VerbFn, session: *mut SessionHandle, request: &[u8]) -> Reply {
    let mut out = junk();
    // SAFETY: `request` is readable for its length, `out` is a writable `Buf`.
    let status = unsafe { verb(session, request.as_ptr(), request.len(), &raw mut out) };
    Reply {
        status,
        bytes: take(out),
    }
}

/// The opened session, or the status and error bytes of the refusal.
pub struct Opened {
    pub status: i32,
    pub session: *mut SessionHandle,
    pub error: Vec<u8>,
}

pub fn open_bytes(config: &[u8]) -> Opened {
    let mut session = NonNull::<SessionHandle>::dangling().as_ptr();
    let mut error = junk();
    // SAFETY: `config` is readable for its length; both outputs are writable.
    let status = unsafe {
        ucdk_session_open(
            config.as_ptr(),
            config.len(),
            &raw mut session,
            &raw mut error,
        )
    };
    Opened {
        status,
        session,
        error: take(error),
    }
}

pub fn open(config: &SessionConfig) -> Opened {
    open_bytes(&config.encode_to_vec())
}

/// A config for the fake provider in the cell the direct engine runs use.
pub fn fake_config() -> SessionConfig {
    SessionConfig {
        provider: "fake".to_owned(),
        cell: kit::cell().to_string(),
        ..SessionConfig::default()
    }
}

pub fn open_fake() -> *mut SessionHandle {
    let opened = open(&fake_config());
    assert_eq!(opened.status, 0);
    assert!(!opened.session.is_null());
    opened.session
}

pub fn error_of(bytes: &[u8]) -> Error {
    Error::decode(bytes).expect("an Error message")
}

pub fn service_of(id: &str, port: u32) -> Resource {
    Resource {
        id: id.to_owned(),
        body: Some(Body::Service(Service {
            image: Some(Image {
                source: Some(image::Source::Digest("sha256:aa".to_owned())),
                platform: String::new(),
            }),
            port,
            ..Service::default()
        })),
        uses: Vec::new(),
    }
}

pub fn list_of(resources: Vec<Resource>) -> ResourceList {
    ResourceList {
        resource: resources,
    }
}

/// The fake provider, with a hook that runs at the start of every `observe`.
pub struct Hooked {
    pub fake: Fake,
    pub on_observe: Box<dyn FnMut() + Send>,
    /// Forget the fake's call log after each read, so a long loop of calls does not grow it.
    pub forget: bool,
    /// Panic when the provider is dropped (unless already unwinding).
    pub panic_on_drop: bool,
}

impl Drop for Hooked {
    fn drop(&mut self) {
        assert!(
            !self.panic_on_drop || std::thread::panicking(),
            "the provider panics in its destructor"
        );
    }
}

impl Hooked {
    pub fn new(on_observe: impl FnMut() + Send + 'static) -> Self {
        Self {
            fake: Fake::standard(),
            on_observe: Box::new(on_observe),
            forget: false,
            panic_on_drop: false,
        }
    }
}

impl Hooked {
    fn forgot(&mut self) {
        if self.forget {
            self.fake.clear_calls();
        }
    }
}

impl Provider for Hooked {
    fn hosts(&self, kind: &Kind) -> bool {
        Provider::hosts(&self.fake, kind)
    }

    fn state_class(&self, kind: &Kind) -> StateClass {
        self.fake.state_class(kind)
    }

    fn deletability(&self, kind: &Kind, live: &Live) -> Deletability {
        self.fake.deletability(kind, live)
    }

    fn stamp_mode(&self, kind: &Kind) -> StampMode {
        self.fake.stamp_mode(kind)
    }

    fn observe(
        &mut self,
        node: &Node,
        hint: Option<&PhysicalId>,
    ) -> Result<Observation, ProviderError> {
        (self.on_observe)();
        let observed = self.fake.observe(node, hint);
        self.forgot();
        observed
    }

    fn create(
        &mut self,
        node: &Node,
        stamp: &ucdk_provider::Stamp,
    ) -> Result<Created, ProviderError> {
        self.fake.create(node, stamp)
    }

    fn update(&mut self, node: &Node, live: &Live) -> Result<Updated, ProviderError> {
        self.fake.update(node, live)
    }

    fn delete(&mut self, kind: &Kind, physical: &PhysicalId) -> Result<(), ProviderError> {
        self.fake.delete(kind, physical)
    }

    fn list_owned(&mut self, scope: &OwnerScope) -> Result<Vec<OwnedObject>, ProviderError> {
        let listed = self.fake.list_owned(scope);
        self.forgot();
        listed
    }

    fn list_stamped(&mut self, scope: &CellScope) -> Result<Vec<OwnedObject>, ProviderError> {
        let listed = self.fake.list_stamped(scope);
        self.forgot();
        listed
    }
}

pub fn null_session() -> *mut SessionHandle {
    ptr::null_mut()
}
