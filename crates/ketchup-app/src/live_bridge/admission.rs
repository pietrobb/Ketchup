//! Admission of loopback connections on the window's endpoints: a bounded
//! number at once, and idle strangers cannot keep a credential's holder out.
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicUsize, Ordering},
    },
};

/// Where a connection is in its handshake; acceptor and connection race for
/// it once, so a connection that showed its credential is never evicted.
pub(super) const PENDING: u8 = 0;
pub(super) const AUTHENTICATED: u8 = 1;
pub(super) const EVICTED: u8 = 2;

pub(super) struct ConnectionPermit(Arc<AtomicUsize>);

impl ConnectionPermit {
    fn claim(active: &Arc<AtomicUsize>, limit: usize) -> Option<Self> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < limit).then_some(count + 1)
            })
            .ok()
            .map(|_| Self(Arc::clone(active)))
    }
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Connections that have not yet shown their credential, oldest first. When
/// every slot is taken, a new connection evicts the oldest of them instead of
/// being turned away. An evicted connection sees it within one poll of
/// reading and closes, so up to twice `limit` are alive meanwhile.
#[derive(Default)]
pub(super) struct Pending(VecDeque<Arc<AtomicU8>>);

impl Pending {
    /// Evicts the oldest connection still waiting for its credential.
    fn evict_oldest(&mut self) -> bool {
        while let Some(state) = self.0.pop_front() {
            // Held by this list alone, its connection already ended.
            if Arc::strong_count(&state) > 1
                && state
                    .compare_exchange(PENDING, EVICTED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                return true;
            }
        }
        false
    }

    pub(super) fn admit(
        &mut self,
        active: &Arc<AtomicUsize>,
        limit: usize,
    ) -> Option<(ConnectionPermit, Arc<AtomicU8>)> {
        self.0.retain(|state| {
            Arc::strong_count(state) > 1 && state.load(Ordering::Acquire) == PENDING
        });
        let permit = ConnectionPermit::claim(active, limit).or_else(|| {
            self.evict_oldest()
                .then(|| ConnectionPermit::claim(active, 2 * limit))
                .flatten()
        })?;
        let state = Arc::new(AtomicU8::new(PENDING));
        self.0.push_back(Arc::clone(&state));
        Some((permit, state))
    }
}
