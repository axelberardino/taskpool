//! Task pool
//!
//! Bounded pool of tasks
//! Basically a queue of futures.
//!

/// Errors for the task pool
mod errors;
pub use errors::TaskPoolError;

/// Task pool, queue of futures, with a drain mecanism
mod taskpool;
pub use taskpool::TaskPool;

#[cfg(test)]
mod taskpool_test;
