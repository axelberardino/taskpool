use std::{future::Future, pin::Pin, sync::Arc};

use std::num::NonZeroUsize;
use tokio::{
    sync::{Semaphore, mpsc::Sender},
    task::JoinHandle,
};

use super::TaskPoolError;

/// Inner task wraps a function and a special state
enum InnerTask {
    /// Task to execute
    Task(Pin<Box<dyn Future<Output = ()> + Send>>),
    /// Stop the task pool
    Stop,
}

/// Inner handle wraps a task handle and a special state
enum InnerHandle {
    /// Handle which need a completion check
    Handle(JoinHandle<()>),
    /// No more handles to wait
    Stop,
}

/// Bounded pool of tasks
pub struct TaskPool {
    /// Ordering queue to schedule tasks while keeping the order
    ordering_queue: Sender<InnerTask>,
    /// Number of allowed parallel tasks
    concurrency: usize,
    /// Size of the backpressure queue
    queue_size: usize,
}

impl TaskPool {
    /// Create a new `Self`
    #[must_use]
    pub fn new(
        concurrency: NonZeroUsize,
        queue_size: NonZeroUsize,
    ) -> (Self, tokio::sync::oneshot::Receiver<()>) {
        let concurrency = concurrency.get();
        let queue_size = queue_size.get();
        let sem = Arc::new(Semaphore::new(concurrency));

        // Allocate a huge queue to schedule tasks in order
        let (queue_tx, mut queue_rx) = tokio::sync::mpsc::channel::<InnerTask>(queue_size);
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

        // Depop the queue and schedule tasks
        let sem_cpy = Arc::clone(&sem);
        let (ack_tx, mut ack_rx) = tokio::sync::mpsc::unbounded_channel::<InnerHandle>();

        // Main queue
        tokio::spawn(async move {
            while let Some(inner_task) = queue_rx.recv().await {
                let task = match inner_task {
                    InnerTask::Task(task) => task,
                    InnerTask::Stop => break,
                };

                let guard_res = Arc::clone(&sem_cpy).acquire_owned().await;
                let guard = match guard_res {
                    Ok(guard) => guard,
                    Err(err) => {
                        tracing::error!("failed to acquire semaphore, skipping task: {err}");
                        continue;
                    }
                };

                let job = tokio::spawn(async move {
                    let _guard = guard;
                    task.await;
                });
                if let Err(err) = ack_tx.send(InnerHandle::Handle(job)) {
                    tracing::warn!("issue occured while send task handle: {err}");
                }
            }

            // Just so we know when to stop consuming handles
            if let Err(err) = ack_tx.send(InnerHandle::Stop) {
                tracing::warn!("issue occured while send task handle: {err}");
            }
        });

        // Acknowledgment queue
        tokio::spawn(async move {
            // Try to consume all remaining jobs, to ensure all of them has been
            // executed.
            while let Some(inner_handle) = ack_rx.recv().await {
                match inner_handle {
                    InnerHandle::Handle(ack_handle) => {
                        if let Err(err) = ack_handle.await {
                            tracing::warn!("issue occured while waiting for task: {err}");
                        }
                    }
                    InnerHandle::Stop => break,
                }
            }

            // The queue has been fully drained, and all tasks has been executed
            if let Err(()) = stop_tx.send(()) {
                tracing::warn!(
                    "issue occured while trying to trigger the end of the task pool drain"
                );
            }
        });

        (
            Self {
                ordering_queue: queue_tx,
                concurrency,
                queue_size,
            },
            stop_rx,
        )
    }

    /// Get the concurrency of the pool
    #[must_use]
    pub const fn concurrency(&self) -> usize {
        self.concurrency
    }

    /// Get the queue size of the pool
    #[must_use]
    pub const fn queue_size(&self) -> usize {
        self.queue_size
    }

    /// Spawn a task in the pool with the default timeout.
    /// If a task can't be inserted, it will wait until it can.
    /// It's usually better to use `spawn_with_timeout`, to avoid locking.
    ///
    /// # Errors
    ///
    /// Returns an error if fails to schedule (e.g., timeout or channel closed).
    pub async fn spawn(
        &self,
        cb: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), TaskPoolError> {
        let pinned_cb = Box::pin(cb);

        match self.ordering_queue.send(InnerTask::Task(pinned_cb)).await {
            Ok(()) => Ok(()),
            Err(_) => Err(TaskPoolError::FailedToSend),
        }
    }

    /// Spawn a task in the pool with the given timeout.
    /// Timeout is not for the task, but for channel queue insertion.
    ///
    /// # Errors
    ///
    /// Returns an error if fails to schedule (e.g., timeout or channel closed).
    pub async fn spawn_with_timeout(
        &self,
        cb: impl Future<Output = ()> + Send + 'static,
        timeout: std::time::Duration,
    ) -> Result<(), TaskPoolError> {
        let pinned_cb = Box::pin(cb);

        match self
            .ordering_queue
            .send_timeout(InnerTask::Task(pinned_cb), timeout)
            .await
        {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::SendTimeoutError::Timeout(_)) => {
                // Because the return type of the timeout include the given
                // callback type, it forces us to be constraint by "Sync",
                // without that, it's possible to directly send anonymous
                // callback (like async{}).
                Err(TaskPoolError::SendTimeout)
            }
            Err(tokio::sync::mpsc::error::SendTimeoutError::Closed(_)) => {
                // _task_future is the Pin<Box<dyn Future + Send>>.
                // It's not Sync and not Debug, so we can't easily include it in the error.
                // We simply acknowledge the channel was closed.
                Err(TaskPoolError::ChannelClosed)
            }
        }
    }

    /// Trigger a stop by adding a special event in the pool.
    /// Everything after this marker will not be consumed and the consuming of
    /// the queue will be stopped.
    ///
    /// # Errors
    ///
    /// Returns an error if fails to schedule.
    pub async fn trigger_stop(&self) -> Result<(), TaskPoolError> {
        // The only inner error possible is "channel close".
        // So let's not depend on the return type and write the same thing
        // manually. By doing that, we're not relying on the inner callback to
        // be constraint by Sync.
        match self.ordering_queue.send(InnerTask::Stop).await {
            Ok(()) => Ok(()),
            Err(_) => Err(TaskPoolError::ChannelClosed),
        }
    }
}
