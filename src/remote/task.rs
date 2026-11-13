//! Work on a thread of its own, with the result picked up by the UI loop.

use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

/// A job running in the background. Dropping it does not stop the thread;
/// its result is then thrown away.
#[derive(Debug)]
pub struct Task<T> {
    result: Arc<Mutex<Option<T>>>,
    /// The thread, for [`Task::wait`]; taken by the first wait.
    thread: Mutex<Option<thread::JoinHandle<()>>>,
}

impl<T: Send + 'static> Task<T> {
    /// Run `work` on a new thread; `wake` is called when it is done (to
    /// repaint the window).
    pub fn spawn(wake: impl FnOnce() + Send + 'static, work: impl FnOnce() -> T + Send + 'static) -> Self {
        let result = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&result);
        let thread = thread::spawn(move || {
            let value = work();
            *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
            wake();
        });
        Task { result, thread: Mutex::new(Some(thread)) }
    }

    /// The result once the work is done (handed out once).
    pub fn take(&self) -> Option<T> {
        self.result.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    /// Block until the work is done and hand out its result (`None` when
    /// it was taken already or the work panicked). Freezes the window while
    /// it waits, so only for steps that cannot go on without the result.
    pub fn wait(&self) -> Option<T> {
        let thread = self.thread.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(thread) = thread {
            thread.join().ok()?;
        }
        self.take()
    }
}

/// Run `work` in the background, repainting `ctx` when it is done.
pub fn spawn_with_repaint<T: Send + 'static>(
    ctx: &eframe::egui::Context,
    work: impl FnOnce() -> T + Send + 'static,
) -> Task<T> {
    let ctx = ctx.clone();
    Task::spawn(move || ctx.request_repaint(), work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn hands_out_the_result_once() {
        let (sender, receiver) = mpsc::channel();
        let task = Task::spawn(move || sender.send(()).unwrap(), || 6 * 7);
        receiver.recv().unwrap();
        assert_eq!(task.take(), Some(42));
        assert_eq!(task.take(), None);
    }

    #[test]
    fn waits_for_the_result() {
        let task = Task::spawn(|| {}, || 6 * 7);
        assert_eq!(task.wait(), Some(42));
        let panicked: Task<u8> = Task::spawn(|| {}, || panic!("broken work"));
        assert_eq!(panicked.wait(), None);
    }
}
