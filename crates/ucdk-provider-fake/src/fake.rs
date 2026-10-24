// SPDX-License-Identifier: Apache-2.0
//! The fake provider.

use std::collections::BTreeMap;

use ucdk_provider::node::digest_of;
use ucdk_provider::{
    Created, Deletability, Kind, Live, Node, Observation, OwnedObject, OwnerScope, PhysicalId,
    Provider, ProviderError, Refusal, StampEvidence, StampMode, StateClass, Updated,
};
use ucdk_provider::{Digest, Stamp};

use crate::config::{FakeConfig, Faults};
use crate::log::{Call, Op};
use crate::store::{Object, labels_of};

/// An in-memory cloud. Objects are ordered by physical id (`fake-000001`, in creation order).
#[derive(Debug)]
pub struct Fake {
    pub(crate) config: FakeConfig,
    pub(crate) objects: BTreeMap<PhysicalId, Object>,
    pub(crate) calls: Vec<Call>,
    next_id: u64,
    mutations: u64,
    reads: u64,
}

impl Fake {
    pub fn new(config: FakeConfig) -> Self {
        let mut fake = Self {
            config,
            objects: BTreeMap::new(),
            calls: Vec::new(),
            next_id: 0,
            mutations: 0,
            reads: 0,
        };
        for f in fake.config.faults.foreign.clone() {
            fake.add_foreign(&f.kind, &f.name, f.marker);
        }
        fake
    }

    /// The hosted shapes with no faults.
    pub fn standard() -> Self {
        Self::new(FakeConfig::standard())
    }

    /// The faults still to fire. A test arms one between calls through this; the constructor
    /// data in [`Faults`] is the usual way.
    pub fn faults_mut(&mut self) -> &mut Faults {
        &mut self.config.faults
    }

    /// The call log, oldest first.
    pub fn calls(&self) -> &[Call] {
        &self.calls
    }

    /// How many live objects the store holds, hidden ones included.
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Adds an object this cell did not create, visible at once and carrying no stamp of
    /// ours: another tool's `marker` when given, else nothing. Not logged and not counted as
    /// a mutation: it is the world the test starts from.
    pub fn add_foreign(&mut self, kind: &Kind, name: &str, marker: Option<String>) -> PhysicalId {
        self.insert(Object {
            kind: kind.clone(),
            name: name.to_owned(),
            base: name.to_owned(),
            labels: BTreeMap::new(),
            fields: Vec::new(),
            unmodelled: BTreeMap::from([("origin".to_owned(), "external".to_owned())]),
            marker,
            visible_at: 0,
        })
    }

    /// Sets a field the catalog does not model, out of band. False when there is no such
    /// object.
    pub fn set_unmodelled(&mut self, physical: &PhysicalId, key: &str, value: &str) -> bool {
        let found = self.objects.get_mut(physical);
        if let Some(object) = found {
            object.unmodelled.insert(key.to_owned(), value.to_owned());
            return true;
        }
        false
    }

    /// Sets (`Some`) or removes (`None`) one label, out of band. False when there is no such
    /// object.
    pub fn set_label(&mut self, physical: &PhysicalId, key: &str, value: Option<&str>) -> bool {
        let Some(object) = self.objects.get_mut(physical) else {
            return false;
        };
        match value {
            Some(v) => object.labels.insert(key.to_owned(), v.to_owned()),
            None => object.labels.remove(key),
        };
        true
    }

    /// Renames an object out of band; a stamp encoded in its name stays. False when there is
    /// no such object.
    pub fn rename(&mut self, physical: &PhysicalId, new_base: &str) -> bool {
        let tagless = self.objects.get(physical).map(|o| self.is_tagless(&o.kind));
        let Some(object) = self.objects.get_mut(physical) else {
            return false;
        };
        let stamp = if tagless == Some(true) {
            Stamp::decode_name(&object.name).1
        } else {
            None
        };
        object.name = stamp.map_or_else(|| new_base.to_owned(), |s| s.encode_name(new_base));
        new_base.clone_into(&mut object.base);
        true
    }

    /// The stamp the object carried in the call that created it, from the call log.
    pub fn birth_stamp(&self, physical: &PhysicalId) -> Option<Stamp> {
        let born = self
            .calls
            .iter()
            .find(|c| c.op == Op::Create && c.physical.as_ref() == Some(physical))?;
        let tagless = born.kind.as_ref().is_some_and(|k| self.is_tagless(k));
        if tagless {
            Stamp::decode_name(&born.name).1
        } else {
            crate::store::stamp_of(&born.labels)
        }
    }

    fn is_tagless(&self, kind: &Kind) -> bool {
        self.config.tagless.contains(kind)
    }

    fn insert(&mut self, object: Object) -> PhysicalId {
        self.next_id += 1;
        let id = PhysicalId::new(format!("fake-{:06}", self.next_id));
        self.objects.insert(id.clone(), object);
        id
    }

    fn record(&mut self, call: Call) {
        self.calls.push(call);
    }

    /// Counts a mutation; true when it is the one `fail_at_call` names.
    fn fail_now(&mut self) -> bool {
        self.mutations += 1;
        self.config.faults.fail_at_call == Some(self.mutations)
    }

    /// Passes a landed mutation's result on, unless an ambiguous success is armed: then the
    /// change stays and the caller is told the call timed out.
    fn reply<T>(&mut self, landed: Result<T, ProviderError>) -> Result<T, ProviderError> {
        match landed {
            Ok(_) if std::mem::take(&mut self.config.faults.ambiguous_success) => {
                Err(ProviderError::Unknown("the reply timed out".to_owned()))
            }
            other => other,
        }
    }

    /// Counts a read; the number of reads made before this one.
    fn read(&mut self) -> u64 {
        self.reads += 1;
        self.reads - 1
    }

    fn live_of(&self, id: &PhysicalId, object: &Object) -> Live {
        Live {
            physical: id.clone(),
            name: object.base.clone(),
            stamp: object.evidence(self.is_tagless(&object.kind)),
            digest: digest_of(&object.kind, &object.fields),
            unmanaged: object.unmodelled.keys().cloned().collect(),
        }
    }

    fn written(&self, node: &Node, stamp: &Stamp) -> (String, BTreeMap<String, String>) {
        if self.is_tagless(node.kind()) {
            (stamp.encode_name(node.name()), BTreeMap::new())
        } else {
            (node.name().to_owned(), labels_of(stamp))
        }
    }

    fn make_object(&self, node: &Node, stamp: &Stamp, visible_at: u64) -> Object {
        let (name, labels) = self.written(node, stamp);
        Object {
            kind: node.kind().clone(),
            name,
            base: node.name().to_owned(),
            labels,
            fields: node.fields().to_vec(),
            unmodelled: BTreeMap::new(),
            marker: None,
            visible_at,
        }
    }

    fn taken(&self, kind: &Kind, base: &str) -> bool {
        self.objects
            .values()
            .any(|o| &o.kind == kind && o.base == base)
    }

    fn try_create(&mut self, node: &Node, stamp: &Stamp) -> Result<PhysicalId, ProviderError> {
        if self.fail_now() {
            return Err(ProviderError::Refused(Refusal::Rejected));
        }
        if self.config.unsupported.contains(node.kind()) {
            return Err(ProviderError::Refused(Refusal::Unsupported));
        }
        let visible_at = self.reads + self.config.faults.read_lag;
        if std::mem::take(&mut self.config.faults.race_next) {
            let racer = self.make_object(node, stamp, visible_at);
            let id = self.insert(racer);
            self.record(self.call_of(Op::Create, node, stamp, Some(id), Ok(())));
        }
        if self.taken(node.kind(), node.name()) {
            return Err(ProviderError::Refused(Refusal::AlreadyExists));
        }
        let object = self.make_object(node, stamp, visible_at);
        Ok(self.insert(object))
    }

    fn call_of(
        &self,
        op: Op,
        node: &Node,
        stamp: &Stamp,
        physical: Option<PhysicalId>,
        outcome: Result<(), ProviderError>,
    ) -> Call {
        let (name, labels) = self.written(node, stamp);
        Call {
            op,
            kind: Some(node.kind().clone()),
            name,
            labels,
            physical,
            outcome,
        }
    }

    fn plain_call(
        op: Op,
        kind: Option<Kind>,
        name: &str,
        physical: Option<PhysicalId>,
        outcome: Result<(), ProviderError>,
    ) -> Call {
        Call {
            op,
            kind,
            name: name.to_owned(),
            labels: BTreeMap::new(),
            physical,
            outcome,
        }
    }

    fn try_update(&mut self, node: &Node, live: &Live) -> Result<Digest, ProviderError> {
        if self.fail_now() {
            return Err(ProviderError::Refused(Refusal::Rejected));
        }
        let Some(object) = self.objects.get_mut(&live.physical) else {
            return Err(ProviderError::Refused(Refusal::NotFound));
        };
        if &object.kind != node.kind() {
            return Err(ProviderError::Permanent(
                "a change of kind is a replace, not an update".to_owned(),
            ));
        }
        object.fields = node.fields().to_vec();
        Ok(node.digest())
    }

    fn try_delete(&mut self, kind: &Kind, physical: &PhysicalId) -> Result<(), ProviderError> {
        if self.fail_now() {
            return Err(ProviderError::Refused(Refusal::Rejected));
        }
        let Some(object) = self.objects.get(physical) else {
            return Ok(());
        };
        if &object.kind != kind {
            return Err(ProviderError::Permanent(
                "the object is of another kind".to_owned(),
            ));
        }
        if self.config.deletion_forbidden.contains(kind) && !object.unmodelled.is_empty() {
            return Err(ProviderError::Refused(Refusal::Rejected));
        }
        self.objects.remove(physical);
        Ok(())
    }
}

impl Provider for Fake {
    fn state_class(&self, kind: &Kind) -> StateClass {
        if self.config.stateful.contains(kind) {
            StateClass::Stateful
        } else {
            StateClass::Stateless
        }
    }

    fn deletability(&self, kind: &Kind, live: &Live) -> Deletability {
        if self.config.deletion_forbidden.contains(kind) && !live.unmanaged.is_empty() {
            Deletability::Undeletable {
                reason: "the object holds data and its kind forbids deletion".to_owned(),
            }
        } else {
            Deletability::Deletable
        }
    }

    fn stamp_mode(&self, kind: &Kind) -> StampMode {
        if self.is_tagless(kind) {
            StampMode::NameEncoded
        } else {
            StampMode::Tag
        }
    }

    fn observe(
        &mut self,
        node: &Node,
        hint: Option<&PhysicalId>,
    ) -> Result<Observation, ProviderError> {
        let read = self.read();
        let found = match hint {
            Some(id) => self.objects.get_key_value(id),
            None => self
                .objects
                .iter()
                .find(|(_, o)| &o.kind == node.kind() && o.base == node.name()),
        };
        let seen = found
            .filter(|(_, o)| o.visible_at <= read && &o.kind == node.kind())
            .map(|(id, o)| self.live_of(id, o));
        let call = Self::plain_call(
            Op::Observe,
            Some(node.kind().clone()),
            node.name(),
            seen.as_ref().map(|l| l.physical.clone()),
            Ok(()),
        );
        self.record(call);
        Ok(seen.map_or(Observation::Absent, Observation::Present))
    }

    fn create(&mut self, node: &Node, stamp: &Stamp) -> Result<Created, ProviderError> {
        let landed = self.try_create(node, stamp);
        let outcome = landed.clone().map(|_| ());
        let physical = landed.clone().ok();
        let call = self.call_of(Op::Create, node, stamp, physical, outcome);
        self.record(call);
        let created = landed.map(|physical| Created {
            physical,
            digest: node.digest(),
        });
        self.reply(created)
    }

    fn update(&mut self, node: &Node, live: &Live) -> Result<Updated, ProviderError> {
        let landed = self.try_update(node, live);
        let call = Self::plain_call(
            Op::Update,
            Some(node.kind().clone()),
            node.name(),
            Some(live.physical.clone()),
            landed.clone().map(|_| ()),
        );
        self.record(call);
        self.reply(landed.map(|digest| Updated { digest }))
    }

    fn delete(&mut self, kind: &Kind, physical: &PhysicalId) -> Result<(), ProviderError> {
        let landed = self.try_delete(kind, physical);
        let call = Self::plain_call(
            Op::Delete,
            Some(kind.clone()),
            "",
            Some(physical.clone()),
            landed.clone(),
        );
        self.record(call);
        self.reply(landed)
    }

    fn list_owned(&mut self, scope: &OwnerScope) -> Result<Vec<OwnedObject>, ProviderError> {
        let read = self.read();
        let owned: Vec<OwnedObject> = self
            .objects
            .iter()
            .filter(|(_, o)| o.visible_at <= read)
            .filter_map(|(id, o)| match o.evidence(self.is_tagless(&o.kind)) {
                StampEvidence::Present(stamp) if stamp.owner == scope.owner => Some(OwnedObject {
                    kind: o.kind.clone(),
                    physical: id.clone(),
                    name: o.base.clone(),
                    stamp,
                }),
                _ => None,
            })
            .collect();
        let call = Self::plain_call(Op::ListOwned, None, scope.owner.as_str(), None, Ok(()));
        self.record(call);
        Ok(owned)
    }
}
