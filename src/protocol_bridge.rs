//! Bridge from observable wake events into `waker_protocol` scheduler messages.
//!
//! This module is available behind the `protocol` feature. It deliberately
//! stays out of the hot observable path: the runtime still emits [`WakeEvent`]
//! values, and callers decide whether to drain those events into protocol
//! scheduler messages or send them through a [`waker_protocol::SchedulerHandle`].

use std::fmt;

use crate::{
    ColdObservableRuntime, ObservableState, ObservableWakeRuntime, ObserverId, TargetTaskId,
    WakeEvent, WakeReasonCode,
};

/// Error returned when a wake event cannot be bridged into `waker_protocol`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolBridgeError {
    /// The observer matched, but it was not configured with a protocol target.
    MissingTarget { observer_id: ObserverId },
    /// The scheduler handle rejected the wake send.
    SchedulerSendFailed(String),
}

impl fmt::Display for ProtocolBridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTarget { observer_id } => {
                write!(f, "wake event for {observer_id:?} has no protocol target")
            }
            Self::SchedulerSendFailed(err) => write!(f, "scheduler wake send failed: {err}"),
        }
    }
}

impl std::error::Error for ProtocolBridgeError {}

impl WakeReasonCode {
    /// Convert this runtime's compact reason code into the protocol reason enum.
    #[must_use]
    pub fn to_protocol_reason(self) -> waker_protocol::WakeReason {
        match self {
            Self::Spawn => waker_protocol::WakeReason::Spawn,
            Self::Manual => waker_protocol::WakeReason::Manual,
            Self::Inbox => waker_protocol::WakeReason::Inbox,
            Self::Timer => waker_protocol::WakeReason::Timer,
            Self::Condition => waker_protocol::WakeReason::Condition,
            Self::Resume => waker_protocol::WakeReason::Resume,
            Self::Qos => waker_protocol::WakeReason::QoS,
            Self::MemoryPressure => waker_protocol::WakeReason::MemoryPressure,
            Self::Cancel => waker_protocol::WakeReason::Cancel,
            Self::Shutdown => waker_protocol::WakeReason::Shutdown,
        }
    }
}

impl TargetTaskId {
    /// Convert this runtime's backend-agnostic target id into a protocol task id.
    #[must_use]
    pub const fn to_protocol_task_id(self) -> waker_protocol::TaskId {
        waker_protocol::TaskId {
            slot: self.slot,
            generation: self.generation,
        }
    }
}

impl WakeEvent {
    /// Return this event's target as a protocol task id.
    pub fn protocol_target(self) -> Result<waker_protocol::TaskId, ProtocolBridgeError> {
        self.target
            .map(TargetTaskId::to_protocol_task_id)
            .ok_or(ProtocolBridgeError::MissingTarget {
                observer_id: self.observer_id,
            })
    }

    /// Return this event's wake reason as a protocol reason.
    #[must_use]
    pub fn protocol_reason(self) -> waker_protocol::WakeReason {
        self.reason.to_protocol_reason()
    }

    /// Convert this event into a protocol wake scheduler message.
    pub fn to_protocol_wake_msg(
        self,
    ) -> Result<waker_protocol::SchedulerMsg, ProtocolBridgeError> {
        Ok(waker_protocol::SchedulerMsg::Wake {
            id: self.protocol_target()?,
            reason: self.protocol_reason(),
        })
    }

    /// Convert this event into a protocol deliver scheduler message using a
    /// caller-provided envelope.
    ///
    /// The observable runtime should not guess how to deliver typed payloads
    /// into an arbitrary protocol task. This helper keeps that boundary clean:
    /// the wake event supplies the target, while the caller supplies the typed
    /// protocol envelope.
    pub fn to_protocol_deliver_msg(
        self,
        envelope: waker_protocol::EnvelopePacket,
    ) -> Result<waker_protocol::SchedulerMsg, ProtocolBridgeError> {
        Ok(waker_protocol::SchedulerMsg::Deliver {
            id: self.protocol_target()?,
            envelope,
        })
    }

    /// Send this event as a wake through a protocol scheduler handle.
    pub fn send_protocol_wake(
        self,
        handle: &waker_protocol::SchedulerHandle,
    ) -> Result<(), ProtocolBridgeError> {
        handle
            .wake(self.protocol_target()?, self.protocol_reason())
            .map_err(ProtocolBridgeError::SchedulerSendFailed)
    }
}

/// Result of converting or sending a batch of wake events into protocol wakes.
#[derive(Default)]
pub struct ProtocolBridgeReport {
    pub scanned: usize,
    pub converted: usize,
    pub sent: usize,
    pub skipped: usize,
    pub errors: Vec<ProtocolBridgeError>,
    pub messages: Vec<waker_protocol::SchedulerMsg>,
}


impl fmt::Debug for ProtocolBridgeReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProtocolBridgeReport")
            .field("scanned", &self.scanned)
            .field("converted", &self.converted)
            .field("sent", &self.sent)
            .field("skipped", &self.skipped)
            .field("errors", &self.errors)
            .field("messages_len", &self.messages.len())
            .finish()
    }
}

impl ProtocolBridgeReport {
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scanned == 0
    }

    #[must_use]
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    #[must_use]
    pub fn take_messages(self) -> Vec<waker_protocol::SchedulerMsg> {
        self.messages
    }
}

/// Convert wake events into protocol wake messages.
#[must_use]
pub fn protocol_wake_messages_from_events<I>(events: I) -> ProtocolBridgeReport
where
    I: IntoIterator<Item = WakeEvent>,
{
    let mut report = ProtocolBridgeReport::default();

    for event in events {
        report.scanned += 1;
        match event.to_protocol_wake_msg() {
            Ok(message) => {
                report.converted += 1;
                report.messages.push(message);
            }
            Err(error) => {
                report.skipped += 1;
                report.errors.push(error);
            }
        }
    }

    report
}

/// Send wake events directly through a protocol scheduler handle.
pub fn send_protocol_wakes_from_events<I>(
    handle: &waker_protocol::SchedulerHandle,
    events: I,
) -> ProtocolBridgeReport
where
    I: IntoIterator<Item = WakeEvent>,
{
    let mut report = ProtocolBridgeReport::default();

    for event in events {
        report.scanned += 1;
        match event.send_protocol_wake(handle) {
            Ok(()) => {
                report.sent += 1;
            }
            Err(error) => {
                report.skipped += 1;
                report.errors.push(error);
            }
        }
    }

    report
}

impl ObservableWakeRuntime {
    /// Drain inline wake events and convert targeted events into protocol wake
    /// messages.
    pub fn drain_inline_protocol_wake_messages(&mut self) -> ProtocolBridgeReport {
        protocol_wake_messages_from_events(self.drain_inline())
    }

    /// Drain condition-cache wake events and convert targeted events into
    /// protocol wake messages.
    pub fn drain_condition_protocol_wake_messages(&mut self) -> ProtocolBridgeReport {
        protocol_wake_messages_from_events(self.drain_condition())
    }

    /// Drain inline wake events and send targeted events through a protocol
    /// scheduler handle.
    pub fn send_inline_protocol_wakes(
        &mut self,
        handle: &waker_protocol::SchedulerHandle,
    ) -> ProtocolBridgeReport {
        send_protocol_wakes_from_events(handle, self.drain_inline())
    }

    /// Drain condition-cache wake events and send targeted events through a
    /// protocol scheduler handle.
    pub fn send_condition_protocol_wakes(
        &mut self,
        handle: &waker_protocol::SchedulerHandle,
    ) -> ProtocolBridgeReport {
        send_protocol_wakes_from_events(handle, self.drain_condition())
    }
}

impl<S: ObservableState> ColdObservableRuntime<S> {
    /// Drain cold inline wake events and convert targeted events into protocol
    /// wake messages.
    pub fn drain_cold_inline_protocol_wake_messages(&mut self) -> ProtocolBridgeReport {
        protocol_wake_messages_from_events(self.drain_cold_inline())
    }

    /// Drain cold condition-cache wake events and convert targeted events into
    /// protocol wake messages.
    pub fn drain_cold_condition_protocol_wake_messages(&mut self) -> ProtocolBridgeReport {
        protocol_wake_messages_from_events(self.drain_cold_condition())
    }

    /// Drain cold inline wake events and send targeted events through a protocol
    /// scheduler handle.
    pub fn send_cold_inline_protocol_wakes(
        &mut self,
        handle: &waker_protocol::SchedulerHandle,
    ) -> ProtocolBridgeReport {
        send_protocol_wakes_from_events(handle, self.drain_cold_inline())
    }

    /// Drain cold condition-cache wake events and send targeted events through a
    /// protocol scheduler handle.
    pub fn send_cold_condition_protocol_wakes(
        &mut self,
        handle: &waker_protocol::SchedulerHandle,
    ) -> ProtocolBridgeReport {
        send_protocol_wakes_from_events(handle, self.drain_cold_condition())
    }
}
