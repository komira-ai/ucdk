// SPDX-License-Identifier: Apache-2.0
//! The fake provider.

use std::collections::BTreeMap;

use ucdk_provider::node::digest_of;
use ucdk_provider::{
    CellScope, Created, Deletability, Kind, Live, Node, Observation, OwnedObject, OwnerScope,
    PhysicalId, Provider, ProviderError, Refusal, SCHEME, StampEvidence, StampMode, StateClass,
    Status, Updated,
};
use ucdk_provider::{Digest, Stamp};

use crate::config::{FakeConfig, FaultsMut};
use crate::log::{Call, Op};
use crate::store::{Object, labels_of};

/// An in-memory cloud. Objects are ordered by physical id (`fake-000001`, in creation order).
#[derive(Debug)]
pub struct Fake {
    pub(crate) config: FakeConfig,
    pub(crate) objects: BTreeMap<PhysicalId, Object>,
    /// Deleted objects that `observe` still reports (read lag), with the read count from
    /// which they are gone. `list_owned` never shows them.
    gone: BTreeMap<PhysicalId, (Object, u64)>,
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
            gone: BTreeMap::new(),
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

    /// The faults a test may arm between calls. `foreign` is not among them: the objects it
    /// lists are added once, by the constructor, so it is set in [`Faults`] and nowhere else.
    pub fn faults_mut(&mut self) -> FaultsMut<'_> {
        let faults = &mut self.config.faults;
        FaultsMut {
            fail_at_call: &mut faults.fail_at_call,
            read_lag: &mut faults.read_lag,
            race_next: &mut faults.race_next,
            ambiguous_success: &mut faults.ambiguous_success,
            ambiguous_at_call: &mut faults.ambiguous_at_call,
            fail_reads: &mut faults.fail_reads,
            create_failed: &mut faults.create_failed,
            converging_reads: &mut faults.converging_reads,
        }
    }

    /// The call log, oldest first.
    pub fn calls(&self) -> &[Call] {
        &self.calls
    }

    /// Forgets the calls logged so far. The store, and what each object was born with, stay.
    pub fn clear_calls(&mut self) {
        self.calls.clear();
    }

    /// Whether the fake hosts the kind: a create of a kind it does not host is refused.
    pub fn hosts(&self, kind: &Kind) -> bool {
        !self.config.unsupported.contains(kind)
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
            born_name: String::new(),
            born_labels: BTreeMap::new(),
            fields: Vec::new(),
            unmodelled: BTreeMap::from([("origin".to_owned(), "external".to_owned())]),
            marker,
            visible_at: 0,
            status: Status::Ready,
            ready_at: 0,
        })
    }

    /// Puts an object in `status` out of band. `Converging` holds until the next `update` of
    /// the object. False when there is no such object.
    pub fn set_status(&mut self, physical: &PhysicalId, status: Status) -> bool {
        let Some(object) = self.objects.get_mut(physical) else {
            return false;
        };
        object.status = status;
        object.ready_at = u64::MAX;
        true
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

    /// The stamp the object was born with: the name and labels it had when it entered the
    /// store, read from the stored object. A stamp set after that (out of band, or by a
    /// provider that patches after create) is not a stamp at birth. `None` when there is no
    /// such object or it was born without one.
    pub fn birth_stamp(&self, physical: &PhysicalId) -> Option<Stamp> {
        let born = self.objects.get(physical)?;
        if self.is_tagless(&born.kind) {
            Stamp::decode_name(&born.born_name).1
        } else {
            crate::store::stamp_of(&born.born_labels)
        }
    }

    fn is_tagless(&self, kind: &Kind) -> bool {
        self.config.tagless.contains(kind)
    }

    fn insert(&mut self, mut object: Object) -> PhysicalId {
        object.born_name.clone_from(&object.name);
        object.born_labels.clone_from(&object.labels);
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
            Ok(_) if self.times_out() => {
                Err(ProviderError::Unknown("the reply timed out".to_owned()))
            }
            other => other,
        }
    }

    /// Whether the mutation just made reports a timeout: the one-shot flag, or the `k`th
    /// mutation.
    fn times_out(&mut self) -> bool {
        let next = std::mem::take(&mut self.config.faults.ambiguous_success);
        next || self.config.faults.ambiguous_at_call == Some(self.mutations)
    }

    /// Counts a read; the number of reads made before this one, or the error the next read
    /// is made to fail with.
    fn read(&mut self) -> Result<u64, ProviderError> {
        self.reads += 1;
        let faults = &mut self.config.faults;
        if faults.fail_reads.is_empty() {
            Ok(self.reads - 1)
        } else {
            Err(faults.fail_reads.remove(0))
        }
    }

    fn live_of(&self, id: &PhysicalId, object: &Object, read: u64) -> Live {
        Live {
            physical: id.clone(),
            name: object.base.clone(),
            stamp: object.evidence(self.is_tagless(&object.kind)),
            status: object.status_at(read),
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
        let faults = &self.config.faults;
        let status = if faults.create_failed {
            Status::Failed
        } else if faults.converging_reads > 0 {
            Status::Converging
        } else {
            Status::Ready
        };
        Object {
            status,
            ready_at: visible_at + faults.converging_reads,
            kind: node.kind().clone(),
            name,
            base: node.name().to_owned(),
            labels,
            born_name: String::new(),
            born_labels: BTreeMap::new(),
            fields: node.fields().to_vec(),
            unmodelled: BTreeMap::new(),
            marker: None,
            visible_at,
        }
    }

    /// A deleted object that a read still reports: the one `hint` names, or the one with the
    /// node's kind and name, while the reads made since its delete are within the read lag.
    fn stale(&self, node: &Node, hint: Option<&PhysicalId>, read: u64) -> Option<Live> {
        self.gone
            .iter()
            .filter(|(id, (o, until))| {
                read < *until
                    && &o.kind == node.kind()
                    && match hint {
                        Some(h) => *id == h,
                        None => o.base == node.name(),
                    }
            })
            .map(|(id, (o, _))| self.live_of(id, o, read))
            .next()
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
        // A kind with no stamp of its own cannot be created yet: its behaviour is not there.
        if !self.hosts(node.kind()) || self.stamp_mode(node.kind()) == StampMode::Embedded {
            return Err(ProviderError::Refused(Refusal::Unsupported));
        }
        // A kind that cannot carry extra labels refuses them; it never stores the stamp without.
        if !stamp.extra.is_empty() && !self.stamp_mode(node.kind()).carries_extra() {
            return Err(ProviderError::Refused(Refusal::Unsupported));
        }
        let visible_at = self.reads + self.config.faults.read_lag;
        // The racer only wins a name that is free: the cloud keeps one object per kind and name.
        let racing = std::mem::take(&mut self.config.faults.race_next);
        if racing && !self.taken(node.kind(), node.name()) {
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
        // An update converges the object: a failed or converging one is working afterwards.
        object.status = Status::Ready;
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
        let stale = object.clone();
        self.objects.remove(physical);
        let lag = self.config.faults.read_lag;
        if lag > 0 {
            self.gone
                .insert(physical.clone(), (stale, self.reads + lag));
        }
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
        if self.config.embedded.contains(kind) {
            StampMode::Embedded
        } else if self.is_tagless(kind) {
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
        let read = match self.read() {
            Ok(read) => read,
            Err(e) => {
                let call = Self::plain_call(
                    Op::Observe,
                    Some(node.kind().clone()),
                    node.name(),
                    None,
                    Err(e.clone()),
                );
                self.record(call);
                return Err(e);
            }
        };
        let found = match hint {
            Some(id) => self.objects.get_key_value(id),
            None => self
                .objects
                .iter()
                .find(|(_, o)| &o.kind == node.kind() && o.base == node.name()),
        };
        let seen = match found {
            Some((id, o)) => {
                (o.visible_at <= read && &o.kind == node.kind()).then(|| self.live_of(id, o, read))
            }
            None => self.stale(node, hint, read),
        };
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
        self.listed(Op::ListOwned, scope.owner.as_str(), |stamp| {
            stamp.owner == scope.owner && stamp.cell == scope.cell
        })
    }

    fn list_stamped(&mut self, scope: &CellScope) -> Result<Vec<OwnedObject>, ProviderError> {
        self.listed(Op::ListStamped, &scope.cell.to_string(), |stamp| {
            stamp.cell == scope.cell && stamp.scheme == SCHEME
        })
    }
}

impl Fake {
    /// One list call: the visible objects whose stamp `keep` accepts, logged under `op` and
    /// `what`.
    fn listed(
        &mut self,
        op: Op,
        what: &str,
        keep: impl Fn(&Stamp) -> bool,
    ) -> Result<Vec<OwnedObject>, ProviderError> {
        let read = match self.read() {
            Ok(read) => read,
            Err(e) => {
                let call = Self::plain_call(op, None, what, None, Err(e.clone()));
                self.record(call);
                return Err(e);
            }
        };
        let owned: Vec<OwnedObject> = self
            .objects
            .iter()
            .filter(|(_, o)| o.visible_at <= read)
            .filter_map(|(id, o)| match o.evidence(self.is_tagless(&o.kind)) {
                StampEvidence::Present(stamp) if keep(&stamp) => Some(OwnedObject {
                    kind: o.kind.clone(),
                    physical: id.clone(),
                    name: o.base.clone(),
                    stamp,
                }),
                _ => None,
            })
            .collect();
        let call = Self::plain_call(op, None, what, None, Ok(()));
        self.record(call);
        Ok(owned)
    }
}
