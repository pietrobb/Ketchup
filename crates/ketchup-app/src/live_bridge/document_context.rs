//! What the bridge forgets when the window's document goes: another one is
//! opened or started.

use super::LiveBridge;
use std::hash::RandomState;
use std::sync::atomic::Ordering;

impl LiveBridge {
    pub(crate) fn invalidate_document_context(&mut self) {
        // The running job was asked of the document that goes: it stops, and
        // one that would change it answers stale_document (review 2026-11-02 MCP-122).
        self.document_epoch = self.document_epoch.wrapping_add(1);
        if let Some(job) = &self.program_check_job {
            job.request_cancelled().store(true, Ordering::Release);
        }
        self.observed = None;
        self.client_states.clear();
        self.pending = None;
        self.batch_jobs.clear();
        self.batch_job_key = RandomState::new();
        self.next_batch_job = 1;
        self.query.invalidate();
        self.image.revoke();
        // The folder an apply gave is that document's: a program of the same
        // file name in the next one does not read its files (R1124og-STEP-901).
        self.program_folder.take_given();
    }
}
