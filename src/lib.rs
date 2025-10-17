//! A simple single-flight implementation for heavy concurrency.
//!
//! Single flight prevents duplicate work by ensuring that only one execution of a function
//! identified by a key is in-flight at any given time. If multiple tasks attempt to execute the
//! same function concurrently, they will all receive the result of the first execution.
//!
//! This is particularly useful in scenarios where the same expensive operation might be requested
//! multiple times, such as fetching data from a remote service or performing complex computations.
//!
//! # Usage
//!
//! Internally, [`SingleFlight`] works like a normal hash map. If a key is not present, the
//! function is executed and the result is broadcasted to all waiting tasks. If the key is already
//! present, the task waits for the result to be broadcasted.
//!
//! To create a new instance of [`SingleFlight`], use [`SingleFlight::new()`].
//!
//! ```
//! use turboflight::SingleFlight;
//!
//! let group: SingleFlight<(), ()> = SingleFlight::new();
//! ```
//!
//! The [`SingleFlight`] type takes two type parameters: the key type `K` and the value type `V`.
//! The key is used to identify the function being executed, while the value is the result of the
//! function. No two functions with the same key will be executed concurrently.
//!
//! To execute a function, use the [`SingleFlight::work`] method.
//!
//! ```
//! use turboflight::SingleFlight;
//! use tokio::time::{self, Duration};
//!
//! # #[tokio::main]
//! # async fn main() {
//! let group = SingleFlight::new();
//!
//! let mut tasks = vec![];
//!
//! for _ in 0..10 {
//!     let group = group.clone();  // The inner state of the group is an Arc so it's cheaply clonable.
//!     tasks.push(tokio::spawn(async move {
//!         group.work(1, async {
//!             println!("Doing work...");
//!             time::sleep(Duration::from_secs(1)).await;
//!             println!("Work done!");
//! 
//!             100
//!         }).await
//!     }));
//!     println!("Task spawned");
//! }
//!
//! let results = futures::future::join_all(tasks).await;
//! 
//! assert!(results.into_iter().all(|res| res.unwrap() == 100));
//! # }
//! ```
//! 
//! The example above spawns 10 tasks that all attempt to execute the same function concurrently.
//! It'll only be ran once. The output will be similar to:
//! 
//! ```txt
//! Task spawned
//! Task spawned
//! Task spawned
//! Task spawned
//! Task spawned
//! Task spawned
//! Task spawned
//! Task spawned
//! Task spawned
//! Task spawned
//! Doing work...
//! Work done!
//! ```
//! 
//! Note that the `Doing work...` are at the end in this example because of race conditions in the
//! spawner. Check `examples/simple.rs` to try it out yourself.
//! 
//! # Cloning
//! 
//! The [`SingleFlight`] struct is cheaply clonable because its inner state is wrapped in an
//! [`Arc`]. This allows you to easily share a single instance of [`SingleFlight`] across multiple
//! tasks or threads without incurring the overhead of deep copies.
//! 
//! # Concurrency
//! 
//! The implementation leverages [`scc::HashIndex`] for efficient concurrent access to the inner
//! state. [`HashIndex`] is a lock-free, concurrent hash map that allows multiple threads to read
//! without blocking each other, making it well-suited for high-concurrency scenarios.
//! 
//! [`SingleFlight`] performs the best compared to other single-flight implementations when
//! concurrency is high. When concurrency is low, you may find other implementations (like
//! [`singleflight-async`]) to be faster.
//! 
//! TL;DR: This implementation is not the fastest when concurrency is low, but the performance does
//! not drop when concurrency is high.
//! 
//! [`singleflight-async`]: https://crates.io/crates/singleflight-async

use std::sync::Arc;

use scc::HashIndex;
use tokio::sync::broadcast::{self, Sender};

/// A single-flight implementation using [`scc::HashIndex`] and [`tokio::sync::broadcast`].
/// 
/// See the [module-level documentation](crate) for more details.
#[derive(Clone, Default)]
pub struct SingleFlight<K, V> {
    inner: Arc<HashIndex<K, Sender<V>>>,
}

impl<K, V> SingleFlight<K, V>
where
    K: std::hash::Hash + Eq + Clone,
    V: Clone,
{
    pub fn new() -> Self {
        Self {
            inner: Arc::new(HashIndex::new()),
        }
    }

    /// Executes the provided asynchronous function `fut` associated with the given `key`.
    /// 
    /// If another execution with the same key is already in progress, this method will wait
    /// for that execution to complete and return its result instead of executing `fut` again.
    /// 
    /// Arguments:
    /// * `key` - A key that uniquely identifies the function being executed.
    /// * `fut` - An asynchronous function that returns a value of type `V`.
    /// 
    /// Returns:
    /// `V` - The result of the function execution.
    pub async fn work<Fut>(&self, key: K, fut: Fut) -> V
    where
        Fut: Future<Output = V>,
    {
        loop {
            if let Some(tx) = self.inner.peek_with(&key, |_k, v| v.clone()) {
                if let Ok(value) = tx.subscribe().recv().await {
                    return value;
                } else {
                    // Continue to spawn a new execution if the sender has been closed.
                }
            }

            let (tx, _rx) = broadcast::channel::<V>(1);

            if self
                .inner
                .insert_async(key.clone(), tx.clone())
                .await
                .is_err()
            {
                continue;
            }

            let value = fut.await;

            self.inner.remove_async(&key).await;

            // Errors are only returned if there are no active receivers, which can be safely
            // ignored.
            tx.send(value.clone()).ok();

            return value;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, atomic::AtomicUsize},
        time::Duration,
    };

    use super::SingleFlight;

    #[tokio::test]
    async fn test_single_flight() {
        let group = SingleFlight::new();

        let result = group.work(1, async { 42 }).await;
        assert_eq!(result, 42);

        let counter = Arc::new(AtomicUsize::new(0));

        for _ in 0..100 {
            let group = group.clone();
            let counter = counter.clone();
            tokio::spawn(async move {
                group
                    .work(2, async {
                        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        42
                    })
                    .await;
            });
        }

        tokio::time::sleep(Duration::from_secs(1)).await;

        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
