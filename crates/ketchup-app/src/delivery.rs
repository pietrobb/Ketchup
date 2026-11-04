//! Handing a worker's result to whoever waits for it (review 2026-11-04 TST-140).

use std::sync::mpsc::{SendError, Sender, SyncSender, TrySendError};

/// Sends a result whose receiver may have gone away: the request was cancelled, the
/// client disconnected or the window closed, and nobody wants the result any more.
pub(crate) trait Deliver<T> {
    fn deliver(&self, value: T);
}

impl<T> Deliver<T> for Sender<T> {
    fn deliver(&self, value: T) {
        // An unbounded send fails only when the receiver is gone.
        let (Ok(()) | Err(SendError(_))) = self.send(value);
    }
}

impl<T> Deliver<T> for SyncSender<T> {
    fn deliver(&self, value: T) {
        match self.try_send(value) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            // A full channel already holds an answer the receiver has not read; this
            // later one is not what it waits for.
            Err(TrySendError::Full(_)) => {
                eprintln!("a result was not delivered: its channel already held one");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Deliver;
    use std::sync::mpsc;

    #[test]
    fn a_result_reaches_a_waiting_receiver_and_is_dropped_without_one() {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.deliver(1);
        sender.deliver(2);
        assert_eq!(receiver.try_recv(), Ok(1));
        drop(receiver);
        sender.deliver(3);

        let (sender, receiver) = mpsc::channel();
        sender.deliver("kept");
        assert_eq!(receiver.recv(), Ok("kept"));
        drop(receiver);
        sender.deliver("unwanted");
    }
}
