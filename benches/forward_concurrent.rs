use lazydns::dns::types::{RecordClass, RecordType};
use lazydns::dns::{Message, Question};
use std::sync::Arc;
use std::time::Instant;

// Simple global allocator wrapper to track allocations and bytes.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct TrackingAlloc {
    alloc_count: AtomicUsize,
    dealloc_count: AtomicUsize,
    alloc_bytes: AtomicUsize,
    dealloc_bytes: AtomicUsize,
}

impl TrackingAlloc {
    const fn new() -> Self {
        Self {
            alloc_count: AtomicUsize::new(0),
            dealloc_count: AtomicUsize::new(0),
            alloc_bytes: AtomicUsize::new(0),
            dealloc_bytes: AtomicUsize::new(0),
        }
    }

    fn snapshot(&self) -> (usize, usize, usize, usize) {
        (
            self.alloc_count.load(Ordering::Relaxed),
            self.dealloc_count.load(Ordering::Relaxed),
            self.alloc_bytes.load(Ordering::Relaxed),
            self.dealloc_bytes.load(Ordering::Relaxed),
        )
    }
}

unsafe impl GlobalAlloc for TrackingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            self.alloc_count.fetch_add(1, Ordering::Relaxed);
            self.alloc_bytes.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        self.dealloc_count.fetch_add(1, Ordering::Relaxed);
        self.dealloc_bytes
            .fetch_add(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        // treat realloc as dealloc of old size + alloc of new size (approx)
        self.dealloc_count.fetch_add(1, Ordering::Relaxed);
        self.dealloc_bytes
            .fetch_add(old_layout.size(), Ordering::Relaxed);
        let new_ptr = unsafe { System.realloc(ptr, old_layout, new_size) };
        if !new_ptr.is_null() {
            self.alloc_count.fetch_add(1, Ordering::Relaxed);
            self.alloc_bytes.fetch_add(new_size, Ordering::Relaxed);
        }
        new_ptr
    }
}

#[global_allocator]
static GLOBAL_TRACKER: TrackingAlloc = TrackingAlloc::new();

async fn process_request(req: Message) {
    // simulate lightweight processing (such as parsing/inspection)
    let _qcount = req.questions().len();
}

async fn process_request_ref(req: &Message) {
    // simulate lightweight processing using a borrowed reference
    let _qcount = req.questions().len();
}

fn stats_mean(durations: &[std::time::Duration]) -> f64 {
    let sum: f64 = durations.iter().map(|d| d.as_secs_f64()).sum();
    sum / durations.len() as f64
}

fn stats_stddev(durations: &[std::time::Duration], mean: f64) -> f64 {
    let var: f64 = durations
        .iter()
        .map(|d| {
            let x = d.as_secs_f64();
            let diff = x - mean;
            diff * diff
        })
        .sum::<f64>()
        / (durations.len() as f64);
    var.sqrt()
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    // Configurations to test
    let upstream_counts = [4usize, 8usize, 16usize, 32usize];
    let concurrent_clients = [10usize, 50usize, 100usize]; // multiple clients per benchmark run
    let iterations_per_client = 20usize; // each client does this many requests
    let samples = 3usize; // repeat each measurement to get average

    // Prepare a moderately complex message
    let mut base_msg = Message::new();
    for i in 0..5 {
        base_msg.add_question(Question::new(
            format!("q{}.example.com", i),
            RecordType::A,
            RecordClass::IN,
        ));
    }

    println!("Running concurrent-clients benchmark");
    println!(
        "samples={}, iterations_per_client={}, worker_threads=8",
        samples, iterations_per_client
    );

    // Prepare a set of simulated RTTs (ms) to vary upstream latencies
    let latency_choices = vec![5u64, 10u64, 50u64];

    for &num_upstreams in &upstream_counts {
        println!("\n=== upstreams = {} ===", num_upstreams);

        for &num_clients in &concurrent_clients {
            println!("\n  clients = {}", num_clients);

            let mut clone_runs = Vec::with_capacity(samples);
            let mut arc_runs = Vec::with_capacity(samples);

            for s in 0..samples {
                // Deep-clone variant: spawn concurrent clients
                let before = GLOBAL_TRACKER.snapshot();
                let start = Instant::now();

                let mut client_tasks = Vec::with_capacity(num_clients);
                for _ in 0..num_clients {
                    let msg = base_msg.clone();
                    let latencies = latency_choices.clone();
                    let num_ups = num_upstreams;
                    let iters = iterations_per_client;

                    let client_task = tokio::spawn(async move {
                        // Each client does multiple request iterations
                        for _ in 0..iters {
                            let mut upstream_tasks = Vec::with_capacity(num_ups);
                            for i in 0..num_ups {
                                let req = msg.clone();
                                let latency = latencies[i % latencies.len()];
                                upstream_tasks.push(tokio::spawn(async move {
                                    // Simulate network RTT
                                    tokio::time::sleep(std::time::Duration::from_millis(latency))
                                        .await;
                                    process_request(req).await
                                }));
                            }
                            // Wait for all upstream tasks to complete
                            for h in upstream_tasks {
                                let _ = h.await;
                            }
                        }
                    });
                    client_tasks.push(client_task);
                }

                // Wait for all clients to complete
                for h in client_tasks {
                    let _ = h.await;
                }

                let dur = start.elapsed();
                let after = GLOBAL_TRACKER.snapshot();
                let allocs = after.0.saturating_sub(before.0);
                let bytes_alloc = after.2.saturating_sub(before.2);
                println!(
                    "    sample {} deep_clone: {:?} | allocs={} bytes={}",
                    s + 1,
                    dur,
                    allocs,
                    bytes_alloc
                );
                clone_runs.push(dur);

                // Arc variant: spawn concurrent clients with Arc<Message>
                let before = GLOBAL_TRACKER.snapshot();
                let start = Instant::now();

                let arc_msg = Arc::new(base_msg.clone());
                let mut client_tasks = Vec::with_capacity(num_clients);
                for _ in 0..num_clients {
                    let msg_arc = Arc::clone(&arc_msg);
                    let latencies = latency_choices.clone();
                    let num_ups = num_upstreams;
                    let iters = iterations_per_client;

                    let client_task = tokio::spawn(async move {
                        // Each client does multiple request iterations
                        for _ in 0..iters {
                            let mut upstream_tasks = Vec::with_capacity(num_ups);
                            for i in 0..num_ups {
                                let req = Arc::clone(&msg_arc);
                                let latency = latencies[i % latencies.len()];
                                upstream_tasks.push(tokio::spawn(async move {
                                    // Simulate network RTT
                                    tokio::time::sleep(std::time::Duration::from_millis(latency))
                                        .await;
                                    process_request_ref(&req).await
                                }));
                            }
                            // Wait for all upstream tasks to complete
                            for h in upstream_tasks {
                                let _ = h.await;
                            }
                        }
                    });
                    client_tasks.push(client_task);
                }

                // Wait for all clients to complete
                for h in client_tasks {
                    let _ = h.await;
                }

                let dur = start.elapsed();
                let after = GLOBAL_TRACKER.snapshot();
                let allocs = after.0.saturating_sub(before.0);
                let bytes_alloc = after.2.saturating_sub(before.2);
                println!(
                    "    sample {} arc_clone:    {:?} | allocs={} bytes={}",
                    s + 1,
                    dur,
                    allocs,
                    bytes_alloc
                );
                arc_runs.push(dur);
            }

            let clone_mean = stats_mean(&clone_runs);
            let arc_mean = stats_mean(&arc_runs);
            let clone_std = stats_stddev(&clone_runs, clone_mean);
            let arc_std = stats_stddev(&arc_runs, arc_mean);

            println!("  Summary clients={}:", num_clients);
            println!("    deep_clone: {:.4}s ± {:.4}s", clone_mean, clone_std);
            println!("    arc_clone:  {:.4}s ± {:.4}s", arc_mean, arc_std);
            println!("    ratio: {:.3}x", clone_mean / arc_mean);
        }
    }
}
