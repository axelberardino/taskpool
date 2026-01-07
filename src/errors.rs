/// Sign in errors
#[derive(thiserror::Error, Debug)]
pub enum TaskPoolError {
    /// Insertion in the queue has timeout, meaning the queue is already full
    /// and is full for too long
    #[error("failed to schedule task in pool: send timeout")]
    SendTimeout,

    /// The channel queue has been closed
    #[error("failed to schedule task in pool: channel closed")]
    ChannelClosed,

    /// Can't send a task
    #[error("failed to schedule task in pool: can't send")]
    FailedToSend,
}
