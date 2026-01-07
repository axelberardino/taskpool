# taskpool

Bounded Tokio task pool with backpressure and ordered scheduling.

The pool accepts async tasks, enforces a maximum concurrency, and buffers
incoming tasks in a bounded queue. When the queue is full, `spawn_with_timeout`
lets you fail fast instead of waiting forever. A drain signal is returned at
construction time so you can wait for all scheduled work to finish after
calling `trigger_stop`.

## Installation

Add `Taskpool` to your `Cargo.toml`:

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
