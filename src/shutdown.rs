use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

const PHASE_RUNNING: u8 = 0;
const PHASE_GRACEFUL: u8 = 1;
const PHASE_IMMEDIATE: u8 = 2;

/// Two-phase shutdown coordinator.
///
/// Phase 0 (Running):   Normal operation.
/// Phase 1 (Graceful):  Stop scheduling new work; let in-flight tasks finish.
/// Phase 2 (Immediate): Force-terminate workers and exit.
#[derive(Clone)]
pub struct ShutdownController {
    phase: Arc<AtomicU8>,
    graceful_notify: Arc<Notify>,
    immediate_notify: Arc<Notify>,
}

impl ShutdownController {
    pub fn new() -> Self {
        Self {
            phase: Arc::new(AtomicU8::new(PHASE_RUNNING)),
            graceful_notify: Arc::new(Notify::new()),
            immediate_notify: Arc::new(Notify::new()),
        }
    }

    pub fn is_graceful(&self) -> bool {
        self.phase.load(Ordering::Acquire) >= PHASE_GRACEFUL
    }

    pub fn is_immediate(&self) -> bool {
        self.phase.load(Ordering::Acquire) >= PHASE_IMMEDIATE
    }

    /// Advance to the next shutdown phase. Returns the new phase.
    pub fn advance(&self) -> u8 {
        let prev = self.phase.fetch_add(1, Ordering::AcqRel);
        let new = (prev + 1).min(PHASE_IMMEDIATE);
        self.phase.store(new, Ordering::Release);
        match new {
            PHASE_GRACEFUL => self.graceful_notify.notify_waiters(),
            PHASE_IMMEDIATE => self.immediate_notify.notify_waiters(),
            _ => {}
        }
        new
    }

    /// Wait until graceful shutdown is requested.
    pub async fn wait_graceful(&self) {
        if self.is_graceful() {
            return;
        }
        self.graceful_notify.notified().await;
    }

    /// Wait until immediate shutdown is requested.
    pub async fn wait_immediate(&self) {
        if self.is_immediate() {
            return;
        }
        self.immediate_notify.notified().await;
    }
}
