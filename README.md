# turboflight

A simple single-flight implementation for heavy concurrency.

## Simple Usage

```rust
use turboflight::SingleFlight;
use tokio::time::{self, Duration};

#[tokio::main]
async fn main() {
    let group = SingleFlight::new();

    let mut tasks = vec![];

    for _ in 0..10 {
        let group = group.clone();  // The inner state of the group is an Arc so it's cheaply clonable.
        tasks.push(tokio::spawn(async move {
            group.work(1, || async {
                println!("Doing work...");
                time::sleep(Duration::from_secs(1)).await;
                println!("Work done!");

                100
            }).await
        }));
        println!("Task spawned");
    }

    let results = futures::future::join_all(tasks).await;

    assert!(results.into_iter().all(|res| res.unwrap() == 100));
}
```

## Documentation

For the full documentation, check [`docs.rs/turboflight`](https://docs.rs/turboflight).
