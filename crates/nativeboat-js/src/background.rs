//! Results of work done on other threads, such as child process output,
//! delivered to JavaScript on the host's thread.
//!
//! JavaScript registers a handler with `host.task(handler)`, passes the
//! returned id to a native that starts the work, and the work reports back
//! through [`Background::send`]. The host delivers the events when its
//! owner calls [`crate::Host::run_background`], after the waker set with
//! [`crate::Host::set_waker`] signalled that something arrived.

use boa_engine::Context;
use std::fmt;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// A value that background work delivers to its handler.
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    /// `undefined`.
    None,
    /// A number.
    Number(f64),
    /// A string.
    Text(String),
    /// A `Uint8Array`.
    Bytes(Vec<u8>),
}

/// An event for the handler of a task.
#[derive(Debug)]
pub(crate) struct Message {
    pub(crate) task: u32,
    pub(crate) event: String,
    pub(crate) payload: Payload,
}

type Wake = Arc<Mutex<Option<Box<dyn Fn() + Send>>>>;

/// Sends events from any thread to the tasks of one host.
#[derive(Clone)]
pub struct Background {
    sender: Sender<Message>,
    wake: Wake,
}

impl fmt::Debug for Background {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Background").finish_non_exhaustive()
    }
}

impl Background {
    pub(crate) fn new(sender: Sender<Message>) -> Background {
        Background {
            sender,
            wake: Arc::default(),
        }
    }

    /// The background channel of the host that owns `context`.
    pub fn of(context: &Context) -> Background {
        context
            .get_data::<Background>()
            .expect("hosts carry a background channel")
            .clone()
    }

    /// Sends `event` with `payload` to the handler of `task` and wakes the
    /// host's thread.
    pub fn send(&self, task: u32, event: &str, payload: Payload) {
        // Without a receiver the host is gone and nobody waits for events.
        let _ = self.sender.send(Message {
            task,
            event: event.to_owned(),
            payload,
        });
        if let Some(wake) = &*self.wake.lock().expect("wakers do not panic") {
            wake();
        }
    }

    pub(crate) fn set_wake(&self, wake: Box<dyn Fn() + Send>) {
        *self.wake.lock().expect("wakers do not panic") = Some(wake);
    }
}
