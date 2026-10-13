//! Auditable event log and replay summaries for observable wake publishing.
//!
//! This module records the *observable boundary* rather than re-running the
//! scheduler. A record captures the compact wake key, payload handle metadata,
//! delivery counters, state transition summary, and optional protocol target.
//! Replay validates monotonic sequence/order and rebuilds aggregate facts without
//! resolving payloads unless the caller explicitly does so elsewhere.

use big_bad_bool::PackedWord;
use std::fmt;

#[cfg(feature = "serialization")]
use serde::{Deserialize, Serialize};

use crate::{
    ColdPublishReport, ObservablePacket, ObservableWakeRuntime, PayloadRef, StateTransition, TargetTaskId, WakeEvent,
    WakeKey64, WakeReport,
};

#[cfg(feature = "protocol")]
use crate::ProtocolBridgeReport;

#[cfg(feature = "tagged-payloads")]
use crate::{ObservableEnvelope, TaggedPayloadRef, TaggedWakeError, TaggedWakeReport};

/// Monotonic log sequence number.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LogSequence(pub u64);

/// Kind of event-log record.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventLogKind {
    HotPublish,
    ColdPublish,
    WakeEvent,
    TaggedPublish,
    ProtocolBridge,
}

/// Plain payload-handle trace. This is intentionally not a pointer.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PayloadTrace {
    pub slot: u32,
    pub generation: u32,
    pub offset: u32,
    pub len: u32,
}

impl From<PayloadRef> for PayloadTrace {
    fn from(value: PayloadRef) -> Self {
        Self {
            slot: value.slot,
            generation: value.generation,
            offset: value.offset,
            len: value.len,
        }
    }
}

/// Optional tagged-payload handle trace.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaggedPayloadTrace {
    pub index: u32,
    pub generation: u32,
    pub check_word: usize,
    pub selector: u16,
    pub prophecy: u8,
    pub tag: u16,
}

#[cfg(feature = "tagged-payloads")]
impl<T, const LOW_BITS: usize> From<TaggedPayloadRef<T, LOW_BITS>> for TaggedPayloadTrace {
    fn from(value: TaggedPayloadRef<T, LOW_BITS>) -> Self {
        Self {
            index: value.index(),
            generation: value.generation(),
            check_word: value.check_word(),
            selector: value.selector_raw(),
            prophecy: value.prophecy_raw(),
            tag: value.tag(),
        }
    }
}

/// State-transition trace stored beside cold publish records.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct StateTransitionTrace {
    pub changed: bool,
    pub field_mask: u16,
}

impl From<StateTransition> for StateTransitionTrace {
    fn from(value: StateTransition) -> Self {
        Self {
            changed: value.changed,
            field_mask: value.field_mask,
        }
    }
}

/// Backend delivery counters recorded for audit/replay summaries.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DeliveryTrace {
    pub scanned: usize,
    pub matched: usize,
    pub inline: usize,
    pub atomic: usize,
    pub condition: usize,
    pub crossbeam: usize,
    pub manual: usize,
}

impl DeliveryTrace {
    #[must_use]
    pub const fn delivered(self) -> usize {
        self.inline + self.atomic + self.condition + self.crossbeam + self.manual
    }
}

impl From<&WakeReport> for DeliveryTrace {
    fn from(value: &WakeReport) -> Self {
        Self {
            scanned: value.scanned,
            matched: value.matched,
            inline: value.inline_delivered,
            atomic: value.atomic_delivered,
            condition: value.condition_delivered,
            crossbeam: value.crossbeam_delivered,
            manual: value.manual_delivered,
        }
    }
}

impl From<&ColdPublishReport> for DeliveryTrace {
    fn from(value: &ColdPublishReport) -> Self {
        Self {
            scanned: value.hot.scanned + value.cold_scanned,
            matched: value.hot.matched + value.cold_key_matched,
            inline: value.hot.inline_delivered + value.cold_inline_delivered,
            atomic: value.hot.atomic_delivered + value.cold_atomic_delivered,
            condition: value.hot.condition_delivered + value.cold_condition_delivered,
            crossbeam: value.hot.crossbeam_delivered,
            manual: value.hot.manual_delivered + value.cold_manual_delivered,
        }
    }
}

/// Target task trace for protocol/audit records.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetTrace {
    pub slot: u32,
    pub generation: u32,
}

impl From<TargetTaskId> for TargetTrace {
    fn from(value: TargetTaskId) -> Self {
        Self {
            slot: value.slot,
            generation: value.generation,
        }
    }
}

/// One immutable audit record.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLogRecord {
    pub seq: LogSequence,
    pub kind: EventLogKind,
    pub key_raw: u64,
    pub payload: Option<PayloadTrace>,
    pub tagged_payload: Option<TaggedPayloadTrace>,
    pub transition: Option<StateTransitionTrace>,
    pub target: Option<TargetTrace>,
    pub delivery: DeliveryTrace,
    pub error_count: usize,
    pub notes: Vec<String>,
}

impl EventLogRecord {
    #[must_use]
    pub fn key(&self) -> Option<WakeKey64> {
        if self.key_raw == 0 {
            return None;
        }
        WakeKey64::try_from_raw_u128(u128::from(self.key_raw))
    }

    #[must_use]
    pub fn delivered(&self) -> usize {
        self.delivery.delivered()
    }

    #[must_use]
    pub fn is_state_change(&self) -> bool {
        self.transition.map(|t| t.changed).unwrap_or(false)
    }
}

/// Append-only observable event log.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObservableEventLog {
    next_seq: u64,
    records: Vec<EventLogRecord>,
}

impl ObservableEventLog {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    #[must_use]
    pub fn records(&self) -> &[EventLogRecord] {
        &self.records
    }

    pub fn clear(&mut self) {
        self.next_seq = 0;
        self.records.clear();
    }

    #[must_use]
    fn allocate_seq(&mut self) -> LogSequence {
        let seq = LogSequence(self.next_seq);
        self.next_seq = self.next_seq.wrapping_add(1);
        seq
    }

    pub fn push(&mut self, mut record: EventLogRecord) -> LogSequence {
        let seq = self.allocate_seq();
        record.seq = seq;
        self.records.push(record);
        seq
    }

    pub fn append_hot_publish(&mut self, packet: ObservablePacket, report: &WakeReport) -> LogSequence {
        self.push(EventLogRecord {
            seq: LogSequence(0),
            kind: EventLogKind::HotPublish,
            key_raw: packet.key.raw_storage(),
            payload: Some(packet.payload.into()),
            tagged_payload: None,
            transition: None,
            target: None,
            delivery: DeliveryTrace::from(report),
            error_count: report.errors.len(),
            notes: report.errors.iter().map(ToString::to_string).collect(),
        })
    }

    pub fn append_cold_publish(
        &mut self,
        key: WakeKey64,
        payload: PayloadRef,
        report: &ColdPublishReport,
    ) -> LogSequence {
        let mut notes: Vec<String> = report.hot.errors.iter().map(ToString::to_string).collect();
        notes.extend(report.errors.iter().map(ToString::to_string));

        self.push(EventLogRecord {
            seq: LogSequence(0),
            kind: EventLogKind::ColdPublish,
            key_raw: key.raw_storage(),
            payload: Some(payload.into()),
            tagged_payload: None,
            transition: Some(report.transition.into()),
            target: None,
            delivery: DeliveryTrace::from(report),
            error_count: report.hot.errors.len() + report.errors.len(),
            notes,
        })
    }

    pub fn append_wake_event(&mut self, kind: EventLogKind, event: WakeEvent) -> LogSequence {
        self.push(EventLogRecord {
            seq: LogSequence(0),
            kind,
            key_raw: event.key.raw_storage(),
            payload: Some(event.payload.into()),
            tagged_payload: None,
            transition: None,
            target: event.target.map(Into::into),
            delivery: DeliveryTrace {
                matched: 1,
                inline: if event.profile == crate::WakeProfileCode::InlineQueue { 1 } else { 0 },
                atomic: if event.profile == crate::WakeProfileCode::AtomicEpoch { 1 } else { 0 },
                condition: if event.profile == crate::WakeProfileCode::ConditionCache { 1 } else { 0 },
                crossbeam: if event.profile == crate::WakeProfileCode::CrossbeamQueue { 1 } else { 0 },
                manual: if event.profile == crate::WakeProfileCode::Manual { 1 } else { 0 },
                ..DeliveryTrace::default()
            },
            error_count: 0,
            notes: Vec::new(),
        })
    }

    pub fn append_wake_events<I>(&mut self, kind: EventLogKind, events: I) -> Vec<LogSequence>
    where
        I: IntoIterator<Item = WakeEvent>,
    {
        events
            .into_iter()
            .map(|event| self.append_wake_event(kind, event))
            .collect()
    }

    #[cfg(feature = "tagged-payloads")]
    pub fn append_tagged_publish<T, const LOW_BITS: usize>(
        &mut self,
        envelope: ObservableEnvelope<TaggedPayloadRef<T, LOW_BITS>>,
        report: &TaggedWakeReport<T, LOW_BITS>,
    ) -> LogSequence {
        self.push(EventLogRecord {
            seq: LogSequence(0),
            kind: EventLogKind::TaggedPublish,
            key_raw: envelope.key.raw_storage(),
            payload: None,
            tagged_payload: Some(envelope.payload.into()),
            transition: None,
            target: report.events.first().and_then(|event| event.target.map(Into::into)),
            delivery: DeliveryTrace {
                scanned: report.scanned,
                matched: report.matched,
                inline: report.inline_delivered,
                atomic: report.atomic_delivered,
                condition: report.condition_delivered,
                manual: report.manual_delivered,
                crossbeam: 0,
            },
            error_count: report.errors.len(),
            notes: report.errors.iter().map(tagged_error_note).collect(),
        })
    }

    #[cfg(feature = "protocol")]
    pub fn append_protocol_bridge(&mut self, report: &ProtocolBridgeReport) -> LogSequence {
        self.push(EventLogRecord {
            seq: LogSequence(0),
            kind: EventLogKind::ProtocolBridge,
            key_raw: 0,
            payload: None,
            tagged_payload: None,
            transition: None,
            target: first_protocol_target(report),
            delivery: DeliveryTrace {
                scanned: report.scanned,
                matched: report.converted + report.sent,
                inline: report.converted + report.sent,
                ..DeliveryTrace::default()
            },
            error_count: report.errors.len(),
            notes: report.errors.iter().map(ToString::to_string).collect(),
        })
    }

    pub fn replay_summary(&self) -> Result<EventLogReplaySummary, EventLogReplayError> {
        replay_records(&self.records)
    }

    /// Alias for [`ObservableEventLog::replay_summary`].
    ///
    /// The release-oriented API and examples use `log().replay()` because it is
    /// the natural verb users expect after appending audit records.
    pub fn replay(&self) -> Result<EventLogReplaySummary, EventLogReplayError> {
        self.replay_summary()
    }
}



impl ObservableEventLog {
    /// Rebuild a log from already materialized records after replay validation.
    pub fn from_records(records: Vec<EventLogRecord>) -> Result<Self, EventLogReplayError> {
        replay_records(&records)?;
        let next_seq = records
            .last()
            .map(|record| record.seq.0.wrapping_add(1))
            .unwrap_or(0);

        Ok(Self { next_seq, records })
    }
}

#[cfg(feature = "serialization")]
pub const EVENT_LOG_EXPORT_VERSION: u16 = 1;

/// Stable export container for durable observable event logs.
#[cfg(feature = "serialization")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLogSnapshot {
    pub version: u16,
    pub next_seq: u64,
    pub records: Vec<EventLogRecord>,
}

#[cfg(feature = "serialization")]
impl EventLogSnapshot {
    #[must_use]
    pub fn new(next_seq: u64, records: Vec<EventLogRecord>) -> Self {
        Self {
            version: EVENT_LOG_EXPORT_VERSION,
            next_seq,
            records,
        }
    }
}

/// Serialization/import/export error for durable event logs.
#[cfg(feature = "serialization")]
#[derive(Debug)]
pub enum EventLogSerializationError {
    UnsupportedVersion { expected: u16, found: u16 },
    Replay(EventLogReplayError),
    Json(serde_json::Error),
    Io(std::io::Error),
}

#[cfg(feature = "serialization")]
impl fmt::Display for EventLogSerializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { expected, found } => write!(
                f,
                "unsupported observable event log version: expected={expected}, found={found}"
            ),
            Self::Replay(err) => write!(f, "invalid observable event log replay: {err}"),
            Self::Json(err) => write!(f, "observable event log JSON error: {err}"),
            Self::Io(err) => write!(f, "observable event log I/O error: {err}"),
        }
    }
}

#[cfg(feature = "serialization")]
impl std::error::Error for EventLogSerializationError {}

#[cfg(feature = "serialization")]
impl From<EventLogReplayError> for EventLogSerializationError {
    fn from(value: EventLogReplayError) -> Self {
        Self::Replay(value)
    }
}

#[cfg(feature = "serialization")]
impl From<serde_json::Error> for EventLogSerializationError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[cfg(feature = "serialization")]
impl From<std::io::Error> for EventLogSerializationError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[cfg(feature = "serialization")]
impl ObservableEventLog {
    #[must_use]
    pub fn snapshot(&self) -> EventLogSnapshot {
        EventLogSnapshot::new(self.next_seq, self.records.clone())
    }

    pub fn from_snapshot(snapshot: EventLogSnapshot) -> Result<Self, EventLogSerializationError> {
        if snapshot.version != EVENT_LOG_EXPORT_VERSION {
            return Err(EventLogSerializationError::UnsupportedVersion {
                expected: EVENT_LOG_EXPORT_VERSION,
                found: snapshot.version,
            });
        }

        replay_records(&snapshot.records)?;
        let min_next_seq = snapshot
            .records
            .last()
            .map(|record| record.seq.0.wrapping_add(1))
            .unwrap_or(0);

        Ok(Self {
            next_seq: snapshot.next_seq.max(min_next_seq),
            records: snapshot.records,
        })
    }

    pub fn to_json_string(&self) -> Result<String, EventLogSerializationError> {
        Ok(serde_json::to_string(&self.snapshot())?)
    }

    pub fn to_json_pretty_string(&self) -> Result<String, EventLogSerializationError> {
        Ok(serde_json::to_string_pretty(&self.snapshot())?)
    }

    pub fn from_json_str(input: &str) -> Result<Self, EventLogSerializationError> {
        let snapshot: EventLogSnapshot = serde_json::from_str(input)?;
        Self::from_snapshot(snapshot)
    }

    pub fn to_jsonl_string(&self) -> Result<String, EventLogSerializationError> {
        let mut out = String::new();
        for record in &self.records {
            out.push_str(&serde_json::to_string(record)?);
            out.push('\n');
        }
        Ok(out)
    }

    pub fn from_jsonl_str(input: &str) -> Result<Self, EventLogSerializationError> {
        let mut records = Vec::new();
        for line in input.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            records.push(serde_json::from_str::<EventLogRecord>(trimmed)?);
        }
        Ok(Self::from_records(records)?)
    }

    pub fn export_json_path(&self, path: impl AsRef<std::path::Path>) -> Result<(), EventLogSerializationError> {
        std::fs::write(path, self.to_json_pretty_string()?)?;
        Ok(())
    }

    pub fn import_json_path(path: impl AsRef<std::path::Path>) -> Result<Self, EventLogSerializationError> {
        Self::from_json_str(&std::fs::read_to_string(path)?)
    }

    pub fn export_jsonl_path(&self, path: impl AsRef<std::path::Path>) -> Result<(), EventLogSerializationError> {
        std::fs::write(path, self.to_jsonl_string()?)?;
        Ok(())
    }

    pub fn import_jsonl_path(path: impl AsRef<std::path::Path>) -> Result<Self, EventLogSerializationError> {
        Self::from_jsonl_str(&std::fs::read_to_string(path)?)
    }
}

#[cfg(feature = "tagged-payloads")]
fn tagged_error_note(error: &TaggedWakeError) -> String {
    error.to_string()
}

#[cfg(feature = "protocol")]
fn first_protocol_target(report: &ProtocolBridgeReport) -> Option<TargetTrace> {
    report.messages.iter().find_map(|message| match message {
        waker_protocol::SchedulerMsg::Wake { id, .. }
        | waker_protocol::SchedulerMsg::Deliver { id, .. }
        | waker_protocol::SchedulerMsg::Checkpoint { id, .. }
        | waker_protocol::SchedulerMsg::Capitulate { id, .. }
        | waker_protocol::SchedulerMsg::Cancel { id } => Some(TargetTrace {
            slot: id.slot,
            generation: id.generation,
        }),
        waker_protocol::SchedulerMsg::Shutdown => None,
    })
}

/// Replay aggregate over an observable event log.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EventLogReplaySummary {
    pub records: usize,
    pub hot_publishes: usize,
    pub cold_publishes: usize,
    pub tagged_publishes: usize,
    pub wake_events: usize,
    pub protocol_bridges: usize,
    pub delivered: usize,
    pub state_changes: usize,
    pub errors: usize,
    pub last_key_raw: Option<u64>,
    pub last_target: Option<TargetTrace>,
}

/// Replay validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventLogReplayError {
    NonMonotonicSequence { previous: LogSequence, next: LogSequence },
    InvalidWakeKey { seq: LogSequence, raw: u64 },
}

impl fmt::Display for EventLogReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonMonotonicSequence { previous, next } => write!(
                f,
                "event log sequence is not monotonic: previous={previous:?}, next={next:?}"
            ),
            Self::InvalidWakeKey { seq, raw } => {
                write!(f, "event log record {seq:?} contains invalid wake key {raw:#x}")
            }
        }
    }
}

impl std::error::Error for EventLogReplayError {}

#[must_use]
pub fn empty_replay_summary() -> EventLogReplaySummary {
    EventLogReplaySummary::default()
}

pub fn replay_records(records: &[EventLogRecord]) -> Result<EventLogReplaySummary, EventLogReplayError> {
    let mut summary = EventLogReplaySummary::default();
    let mut previous: Option<LogSequence> = None;

    for record in records {
        if let Some(prev) = previous {
            if record.seq <= prev {
                return Err(EventLogReplayError::NonMonotonicSequence {
                    previous: prev,
                    next: record.seq,
                });
            }
        }
        previous = Some(record.seq);

        if record.key_raw != 0 && WakeKey64::try_from_raw_u128(u128::from(record.key_raw)).is_none() {
            return Err(EventLogReplayError::InvalidWakeKey {
                seq: record.seq,
                raw: record.key_raw,
            });
        }

        summary.records += 1;
        summary.delivered += record.delivered();
        summary.errors += record.error_count;
        if record.key_raw != 0 {
            summary.last_key_raw = Some(record.key_raw);
        }
        if let Some(target) = record.target {
            summary.last_target = Some(target);
        }
        if record.is_state_change() {
            summary.state_changes += 1;
        }

        match record.kind {
            EventLogKind::HotPublish => summary.hot_publishes += 1,
            EventLogKind::ColdPublish => summary.cold_publishes += 1,
            EventLogKind::WakeEvent => summary.wake_events += 1,
            EventLogKind::TaggedPublish => summary.tagged_publishes += 1,
            EventLogKind::ProtocolBridge => summary.protocol_bridges += 1,
        }
    }

    Ok(summary)
}

impl ObservableWakeRuntime {
    /// Publish through the hot runtime and append an audit record for the publish.
    pub fn publish_logged(
        &mut self,
        log: &mut ObservableEventLog,
        packet: ObservablePacket,
    ) -> WakeReport {
        let report = self.publish(packet);
        log.append_hot_publish(packet, &report);
        report
    }

    /// Publish through the adaptive hot dispatch policy and append an audit
    /// record for the publish.
    ///
    /// This is the recommended hot-path helper for callers that want the
    /// runtime to choose scan versus indexed dispatch automatically while still
    /// preserving the event-log boundary.
    pub fn publish_auto_logged(
        &mut self,
        log: &mut ObservableEventLog,
        packet: ObservablePacket,
    ) -> WakeReport {
        let report = self.publish_auto(packet);
        log.append_hot_publish(packet, &report);
        report
    }

    /// Publish through the indexed hot dispatch path and append an audit
    /// record for the publish. Prefer [`publish_auto_logged`](Self::publish_auto_logged)
    /// unless the caller already knows the observer layout is sparse/selective.
    pub fn publish_indexed_logged(
        &mut self,
        log: &mut ObservableEventLog,
        packet: ObservablePacket,
    ) -> WakeReport {
        let report = self.publish_indexed(packet);
        log.append_hot_publish(packet, &report);
        report
    }

    /// Drain inline events and append each drained event to the audit log.
    pub fn drain_inline_logged(&mut self, log: &mut ObservableEventLog) -> Vec<WakeEvent> {
        let events = self.drain_inline();
        log.append_wake_events(EventLogKind::WakeEvent, events.iter().copied());
        events
    }

    /// Drain condition-cache events and append each drained event to the audit log.
    pub fn drain_condition_logged(&mut self, log: &mut ObservableEventLog) -> Vec<WakeEvent> {
        let events = self.drain_condition();
        log.append_wake_events(EventLogKind::WakeEvent, events.iter().copied());
        events
    }

    #[cfg(feature = "tagged-payloads")]
    /// Publish a tagged payload envelope and append a tagged audit record.
    pub fn publish_tagged_logged<T, const LOW_BITS: usize>(
        &mut self,
        log: &mut ObservableEventLog,
        envelope: ObservableEnvelope<TaggedPayloadRef<T, LOW_BITS>>,
    ) -> TaggedWakeReport<T, LOW_BITS> {
        let report = self.publish_tagged(envelope);
        log.append_tagged_publish(envelope, &report);
        report
    }
}

impl<S: crate::ObservableState> crate::ColdObservableRuntime<S> {
    /// Publish through the cold state runtime and append an audit record.
    pub fn publish_stateful_logged(
        &mut self,
        log: &mut ObservableEventLog,
        key: WakeKey64,
        payload: PayloadRef,
        event: S::Event,
    ) -> ColdPublishReport {
        let report = self.publish_stateful(key, payload, event);
        log.append_cold_publish(key, payload, &report);
        report
    }

    /// Publish through the adaptive hot/cold dispatch policy and append a cold
    /// publish audit record.
    ///
    /// This is the recommended stateful helper for server/daemon integrations:
    /// hot routing stays adaptive, cold predicates use the tuned scan/index
    /// policy, and the resulting transition is recorded in one replayable log.
    pub fn publish_stateful_auto_logged(
        &mut self,
        log: &mut ObservableEventLog,
        key: WakeKey64,
        payload: PayloadRef,
        event: S::Event,
    ) -> ColdPublishReport {
        let report = self.publish_stateful_auto(key, payload, event);
        log.append_cold_publish(key, payload, &report);
        report
    }

    /// Publish through the indexed hot/cold paths and append a cold publish
    /// audit record. Prefer
    /// [`publish_stateful_auto_logged`](Self::publish_stateful_auto_logged) unless
    /// the caller knows the observer layout is sparse/selective.
    pub fn publish_stateful_indexed_logged(
        &mut self,
        log: &mut ObservableEventLog,
        key: WakeKey64,
        payload: PayloadRef,
        event: S::Event,
    ) -> ColdPublishReport {
        let report = self.publish_stateful_indexed(key, payload, event);
        log.append_cold_publish(key, payload, &report);
        report
    }

    /// Drain cold inline events and append each drained event to the audit log.
    pub fn drain_cold_inline_logged(&mut self, log: &mut ObservableEventLog) -> Vec<WakeEvent> {
        let events = self.drain_cold_inline();
        log.append_wake_events(EventLogKind::WakeEvent, events.iter().copied());
        events
    }

    /// Drain cold condition-cache events and append each drained event to the audit log.
    pub fn drain_cold_condition_logged(&mut self, log: &mut ObservableEventLog) -> Vec<WakeEvent> {
        let events = self.drain_cold_condition();
        log.append_wake_events(EventLogKind::WakeEvent, events.iter().copied());
        events
    }
}
