#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use std::num::NonZeroUsize;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use tokio::sync::{Mutex, Notify, Semaphore, oneshot};

    use crate::{TaskPool, TaskPoolError};

    fn update_max(current_max: &AtomicUsize, candidate: usize) {
        let mut prev = current_max.load(Ordering::SeqCst);
        while candidate > prev {
            match current_max.compare_exchange(prev, candidate, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break,
                Err(next) => prev = next,
            }
        }
    }

    #[tokio::test]
    async fn tasks_run_in_order_with_single_worker() {
        let (pool, stop_rx) = TaskPool::new(
            NonZeroUsize::new(1).expect("can't init non zero usize"),
            NonZeroUsize::new(10).expect("can't init non zero usize"),
        );
        let results = Arc::new(Mutex::new(Vec::new()));

        for i in 0..5_usize {
            let results = Arc::clone(&results);
            let res = pool
                .spawn_with_timeout(
                    async move {
                        results.lock().await.push(i);
                    },
                    Duration::from_secs(1),
                )
                .await;
            match res {
                Ok(()) => {}
                Err(err) => assert!(false, "expected task {i} to be scheduled, got {err:?}"),
            }
        }

        let stop_res = pool.trigger_stop().await;
        match stop_res {
            Ok(()) => {}
            Err(err) => assert!(false, "expected stop to be scheduled, got {err:?}"),
        }

        let stop_wait = tokio::time::timeout(Duration::from_secs(2), stop_rx).await;
        match stop_wait {
            Ok(Ok(())) => {}
            Ok(Err(_)) => assert!(false, "stop channel dropped before completion"),
            Err(_) => assert!(false, "timed out waiting for the pool to drain"),
        }

        let values = results.lock().await;
        assert_eq!(&*values, &vec![0, 1, 2, 3, 4], "tasks should run in order");
    }

    #[tokio::test]
    async fn respects_max_concurrency() {
        let (pool, stop_rx) = TaskPool::new(
            NonZeroUsize::new(2).expect("can't init non zero usize"),
            NonZeroUsize::new(20).expect("can't init non zero usize"),
        );
        let current = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        for _ in 0..6_usize {
            let current = Arc::clone(&current);
            let max_seen = Arc::clone(&max_seen);
            let res = pool
                .spawn_with_timeout(
                    async move {
                        let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                        update_max(&max_seen, now);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        current.fetch_sub(1, Ordering::SeqCst);
                    },
                    Duration::from_secs(1),
                )
                .await;
            match res {
                Ok(()) => {}
                Err(err) => assert!(
                    false,
                    "expected task to schedule within timeout, got {err:?}"
                ),
            }
        }

        let stop_res = pool.trigger_stop().await;
        match stop_res {
            Ok(()) => {}
            Err(err) => assert!(false, "expected stop to be scheduled, got {err:?}"),
        }

        let stop_wait = tokio::time::timeout(Duration::from_secs(2), stop_rx).await;
        match stop_wait {
            Ok(Ok(())) => {}
            Ok(Err(_)) => assert!(false, "stop channel dropped before completion"),
            Err(_) => assert!(false, "timed out waiting for the pool to drain"),
        }

        let peak = max_seen.load(Ordering::SeqCst);
        assert!(peak <= 2, "expected concurrency <= 2, got {peak}");
        assert!(peak >= 1, "expected at least one task to run");
    }

    #[tokio::test]
    async fn stop_waits_for_running_tasks() {
        let (pool, mut stop_rx) = TaskPool::new(
            NonZeroUsize::new(2).expect("can't init non zero usize"),
            NonZeroUsize::new(20).expect("can't init non zero usize"),
        );
        let notify = Arc::new(Notify::new());

        for _ in 0..2_usize {
            let notify = Arc::clone(&notify);
            let res = pool
                .spawn_with_timeout(
                    async move {
                        notify.notified().await;
                    },
                    Duration::from_secs(1),
                )
                .await;
            match res {
                Ok(()) => {}
                Err(err) => assert!(false, "expected task to be scheduled, got {err:?}"),
            }
        }

        let stop_res = pool.trigger_stop().await;
        match stop_res {
            Ok(()) => {}
            Err(err) => assert!(false, "expected stop to be scheduled, got {err:?}"),
        }

        let early_wait = tokio::time::timeout(Duration::from_millis(50), &mut stop_rx).await;
        match early_wait {
            Err(_) => {}
            Ok(Ok(())) => assert!(false, "pool drained before tasks were released"),
            Ok(Err(_)) => assert!(false, "stop channel dropped before completion"),
        }

        notify.notify_waiters();

        let stop_wait = tokio::time::timeout(Duration::from_secs(2), &mut stop_rx).await;
        match stop_wait {
            Ok(Ok(())) => {}
            Ok(Err(_)) => assert!(false, "stop channel dropped before completion"),
            Err(_) => assert!(false, "timed out waiting for the pool to drain"),
        }
    }

    #[tokio::test]
    async fn stop_prevents_tasks_after_marker() {
        let (pool, stop_rx) = TaskPool::new(
            NonZeroUsize::new(2).expect("can't init non zero usize"),
            NonZeroUsize::new(20).expect("can't init non zero usize"),
        );
        let counter = Arc::new(AtomicUsize::new(0));

        for _ in 0..3_usize {
            let counter = Arc::clone(&counter);
            let res = pool
                .spawn_with_timeout(
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    },
                    Duration::from_secs(1),
                )
                .await;
            match res {
                Ok(()) => {}
                Err(err) => assert!(false, "expected task to be scheduled, got {err:?}"),
            }
        }

        let stop_res = pool.trigger_stop().await;
        match stop_res {
            Ok(()) => {}
            Err(err) => assert!(false, "expected stop to be scheduled, got {err:?}"),
        }

        let counter_late = Arc::clone(&counter);
        let res = pool
            .spawn_with_timeout(
                async move {
                    counter_late.fetch_add(1, Ordering::SeqCst);
                },
                Duration::from_secs(1),
            )
            .await;
        match res {
            Ok(()) => {}
            Err(err) => assert!(false, "expected late task to be enqueued, got {err:?}"),
        }

        let stop_wait = tokio::time::timeout(Duration::from_secs(2), stop_rx).await;
        match stop_wait {
            Ok(Ok(())) => {}
            Ok(Err(_)) => assert!(false, "stop channel dropped before completion"),
            Err(_) => assert!(false, "timed out waiting for the pool to drain"),
        }

        let total = counter.load(Ordering::SeqCst);
        assert_eq!(total, 3, "expected only pre-stop tasks to run, got {total}");
    }

    #[tokio::test]
    async fn spawn_times_out_when_queue_is_full() {
        let (pool, stop_rx) = TaskPool::new(
            NonZeroUsize::new(1).expect("can't init non zero usize"),
            NonZeroUsize::new(10).expect("can't init non zero usize"),
        );
        let gate = Arc::new(Semaphore::new(0));
        let (started_tx, started_rx) = oneshot::channel();
        let mut scheduled = 0_usize;

        let gate_first = Arc::clone(&gate);
        let res = pool
            .spawn_with_timeout(
                async move {
                    if let Err(()) = started_tx.send(()) {
                        return;
                    }
                    if gate_first.acquire_owned().await.is_err() {
                        return;
                    }
                },
                Duration::from_secs(1),
            )
            .await;
        match res {
            Ok(()) => scheduled += 1,
            Err(err) => assert!(false, "expected first task to be scheduled, got {err:?}"),
        }

        match started_rx.await {
            Ok(()) => {}
            Err(_) => assert!(false, "first task never started"),
        }

        let mut saw_timeout = false;
        for _ in 0..20_usize {
            let gate_task = Arc::clone(&gate);
            let res = pool
                .spawn_with_timeout(
                    async move {
                        if gate_task.acquire_owned().await.is_err() {
                            return;
                        }
                    },
                    Duration::from_millis(1),
                )
                .await;
            match res {
                Ok(()) => scheduled += 1,
                Err(TaskPoolError::SendTimeout) => {
                    saw_timeout = true;
                    break;
                }
                Err(err) => assert!(false, "unexpected error while filling queue: {err:?}"),
            }
        }

        assert!(
            saw_timeout,
            "expected to hit send timeout when queue is full"
        );

        gate.add_permits(scheduled);

        let stop_res = pool.trigger_stop().await;
        match stop_res {
            Ok(()) => {}
            Err(err) => assert!(false, "expected stop to be scheduled, got {err:?}"),
        }

        let stop_wait = tokio::time::timeout(Duration::from_secs(2), stop_rx).await;
        match stop_wait {
            Ok(Ok(())) => {}
            Ok(Err(_)) => assert!(false, "stop channel dropped before completion"),
            Err(_) => assert!(false, "timed out waiting for the pool to drain"),
        }
    }

    #[tokio::test]
    async fn tasks_run_with_spawn() {
        let (pool, stop_rx) = TaskPool::new(
            NonZeroUsize::new(1).expect("can't init non zero usize"),
            NonZeroUsize::new(10).expect("can't init non zero usize"),
        );
        let results = Arc::new(Mutex::new(Vec::new()));

        for i in 0..5_usize {
            let results = Arc::clone(&results);
            let res = pool
                .spawn(async move {
                    results.lock().await.push(i);
                })
                .await;
            match res {
                Ok(()) => {}
                Err(err) => assert!(false, "expected task {i} to be scheduled, got {err:?}"),
            }
        }

        let stop_res = pool.trigger_stop().await;
        match stop_res {
            Ok(()) => {}
            Err(err) => assert!(false, "expected stop to be scheduled, got {err:?}"),
        }

        let stop_wait = tokio::time::timeout(Duration::from_secs(2), stop_rx).await;
        match stop_wait {
            Ok(Ok(())) => {}
            Ok(Err(_)) => assert!(false, "stop channel dropped before completion"),
            Err(_) => assert!(false, "timed out waiting for the pool to drain"),
        }

        let values = results.lock().await;
        assert_eq!(&*values, &vec![0, 1, 2, 3, 4], "tasks should run in order");
    }

    #[tokio::test]
    async fn spawn_default_when_queue_is_full() {
        let (pool, stop_rx) = TaskPool::new(
            NonZeroUsize::new(1).expect("can't init non zero usize"),
            NonZeroUsize::new(10).expect("can't init non zero usize"),
        );
        let gate = Arc::new(Semaphore::new(0));
        let (started_tx, started_rx) = oneshot::channel();
        let mut scheduled = 0_usize;

        let gate_first = Arc::clone(&gate);
        let res = tokio::time::timeout(
            Duration::from_millis(10),
            pool.spawn(async move {
                if let Err(()) = started_tx.send(()) {
                    return;
                }
                if gate_first.acquire_owned().await.is_err() {
                    return;
                }
            }),
        )
        .await;
        match res {
            Ok(Ok(())) => scheduled += 1,
            Ok(Err(err)) => assert!(false, "expected first task to be scheduled, got {err:?}"),
            Err(_) => assert!(false, "timed out waiting for the pool to drain"),
        }

        match started_rx.await {
            Ok(()) => {}
            Err(_) => assert!(false, "first task never started"),
        }

        let mut saw_timeout = false;
        for _ in 0..20_usize {
            let gate_task = Arc::clone(&gate);

            let res = tokio::time::timeout(
                Duration::from_millis(10),
                pool.spawn(async move {
                    if gate_task.acquire_owned().await.is_err() {
                        return;
                    }
                }),
            )
            .await;
            match res {
                Ok(Ok(())) => scheduled += 1,
                Ok(Err(err)) => assert!(false, "unexpected error while filling queue: {err:?}"),
                Err(_) => {
                    saw_timeout = true;
                    break;
                }
            }
        }

        assert!(
            saw_timeout,
            "expected to hit send timeout when queue is full"
        );

        gate.add_permits(scheduled);

        let stop_res = pool.trigger_stop().await;
        match stop_res {
            Ok(()) => {}
            Err(err) => assert!(false, "expected stop to be scheduled, got {err:?}"),
        }

        let stop_wait = tokio::time::timeout(Duration::from_secs(2), stop_rx).await;
        match stop_wait {
            Ok(Ok(())) => {}
            Ok(Err(_)) => assert!(false, "stop channel dropped before completion"),
            Err(_) => assert!(false, "timed out waiting for the pool to drain"),
        }
    }
}
