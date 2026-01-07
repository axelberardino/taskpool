# TaskPool

Bounded tokio task pool with backpressure and ordered scheduling.

The pool accepts async tasks, enforces a maximum concurrency, and buffers
incoming tasks in a bounded queue. When the queue is full, `spawn_with_timeout`
lets you fail fast instead of waiting forever. A drain signal is returned at
construction time so you can wait for all scheduled work to finish after
calling `trigger_stop`.

## Installation

Add `TaskPool` to your `Cargo.toml`:

```toml
[dependencies]
bounded-taskpool = "0.1.0" # Replace with the latest version
```

## Usage

Add the crate and use it inside a Tokio runtime:

```rust
use std::num::NonZeroUsize;
use std::time::Duration;
use taskpool::TaskPool;

#[tokio::main]
async fn main() {
    let (pool, drained) = TaskPool::new(
        NonZeroUsize::new(4).expect("non-zero concurrency"),
        NonZeroUsize::new(64).expect("non-zero queue size"),
    );

    for i in 0..10 {
        pool.spawn_with_timeout(
            async move {
                println!("job {i}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            },
            Duration::from_millis(250),
        )
        .await
        .expect("schedule task");
    }

    pool.trigger_stop().await.expect("stop accepted");
    drained.await.expect("pool drained");
}
```

If you want to wait indefinitely for space in the queue, use `spawn` instead of
`spawn_with_timeout`.

The `drained` receiver returned by `TaskPool::new` resolves once the stop marker
is observed (and trigger by `trigger_stop`), all queued tasks before it have
run, and the pool finishes draining. Await it after `trigger_stop` when you need
to know all scheduled work has completed.

## API Reference

The main components of the `TaskPool` API are:
* `TaskPool::new(concurrency, queue_size) -> (TaskPool, drained)`: build a
  bounded pool and receive a `drained` oneshot you can await after calling
  `trigger_stop`.
* `TaskPool::concurrency() -> usize`: return the configured maximum number of
  concurrent tasks.
* `TaskPool::queue_size() -> usize`: return the configured backpressure queue
  size.
* `TaskPool::spawn(task) -> Result<(), TaskPoolError>`: enqueue a task, waiting
  indefinitely for space.
* `TaskPool::spawn_with_timeout(task, timeout) -> Result<(), TaskPoolError>`:
  enqueue a task, failing if the queue remains full past `timeout`.
* `TaskPool::trigger_stop() -> Result<(), TaskPoolError>`: stop consuming the
  queue after a sentinel marker.
* `TaskPoolError`: errors returned when scheduling fails (`SendTimeout`,
  `ChannelClosed`, `FailedToSend`).

## Contributing

Contributions are welcome! Please feel free to submit a pull request or open an
issue on GitHub.

## License

This project is licensed under the MIT License. See the [LICENSE](./LICENSE)
file for details.
