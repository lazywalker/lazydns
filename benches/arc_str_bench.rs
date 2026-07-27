//! Benchmark for Arc<str> optimization
//!
//! Compare clone performance: String vs Arc<str>
//!
//! Run with: cargo bench --bench arc_str_bench

use lazydns::dns::types::{RecordClass, RecordType};
use lazydns::dns::{Message, Question, RData, ResourceRecord};
use std::alloc::{GlobalAlloc, Layout, System};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

// Simple global allocator wrapper to track allocations
struct TrackingAlloc {
    alloc_count: AtomicUsize,
    alloc_bytes: AtomicUsize,
}

impl TrackingAlloc {
    const fn new() -> Self {
        Self {
            alloc_count: AtomicUsize::new(0),
            alloc_bytes: AtomicUsize::new(0),
        }
    }

    fn reset(&self) {
        self.alloc_count.store(0, Ordering::SeqCst);
        self.alloc_bytes.store(0, Ordering::SeqCst);
    }

    fn snapshot(&self) -> (usize, usize) {
        (
            self.alloc_count.load(Ordering::SeqCst),
            self.alloc_bytes.load(Ordering::SeqCst),
        )
    }
}

unsafe impl GlobalAlloc for TrackingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.alloc_count.fetch_add(1, Ordering::Relaxed);
        self.alloc_bytes.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        self.alloc_count.fetch_add(1, Ordering::Relaxed);
        self.alloc_bytes.fetch_add(new_size, Ordering::Relaxed);
        unsafe { System.realloc(ptr, old_layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL_TRACKER: TrackingAlloc = TrackingAlloc::new();

/// Create a realistic DNS message for benchmarking
fn create_test_message() -> Message {
    let mut msg = Message::new();
    msg.set_id(12345);
    msg.set_recursion_desired(true);

    // Add multiple questions (typical DNS lookup)
    for i in 0..3 {
        msg.add_question(Question::new(
            format!("subdomain{}.example.com", i),
            RecordType::A,
            RecordClass::IN,
        ));
    }

    // Add answer records
    for i in 0..5 {
        msg.add_answer(ResourceRecord::new(
            format!("subdomain{}.example.com", i % 3),
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A(Ipv4Addr::new(192, 0, 2, i as u8)),
        ));
    }

    // Add NS records in authority
    for i in 0..2 {
        msg.add_authority(ResourceRecord::new(
            "example.com",
            RecordType::NS,
            RecordClass::IN,
            86400,
            RData::NS(format!("ns{}.example.com", i)),
        ));
    }

    // Add additional A records for glue
    for i in 0..2 {
        msg.add_additional(ResourceRecord::new(
            format!("ns{}.example.com", i),
            RecordType::A,
            RecordClass::IN,
            86400,
            RData::A(Ipv4Addr::new(192, 0, 2, 100 + i as u8)),
        ));
    }

    msg
}

/// Create a more complex message with many records
fn create_large_message() -> Message {
    let mut msg = Message::new();
    msg.set_id(54321);

    // Add question
    msg.add_question(Question::new(
        "mail.example.com",
        RecordType::MX,
        RecordClass::IN,
    ));

    // Add many MX records
    for i in 0..10 {
        msg.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::MX,
            RecordClass::IN,
            3600,
            RData::mx(10 + i, format!("mail{}.example.com", i)),
        ));
    }

    // Add CNAME chain
    for i in 0..5 {
        msg.add_answer(ResourceRecord::new(
            format!("alias{}.example.com", i),
            RecordType::CNAME,
            RecordClass::IN,
            3600,
            RData::CNAME(format!("target{}.example.com", i)),
        ));
    }

    // Add TXT records
    for i in 0..3 {
        msg.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::TXT,
            RecordClass::IN,
            3600,
            RData::TXT(vec![format!("v=spf1 include:_spf{}.example.com ~all", i)]),
        ));
    }

    msg
}

/// Benchmark statistics
struct BenchResult {
    name: String,
    iterations: usize,
    total_time: Duration,
    alloc_count: usize,
    alloc_bytes: usize,
}

impl BenchResult {
    fn avg_time_ns(&self) -> f64 {
        self.total_time.as_nanos() as f64 / self.iterations as f64
    }

    fn print(&self) {
        println!(
            "  {}: {:>10} iters | {:>10.2} ns/iter | {:>8} allocs | {:>10} bytes",
            self.name,
            self.iterations,
            self.avg_time_ns(),
            self.alloc_count,
            self.alloc_bytes
        );
    }
}

/// Run a clone benchmark
fn bench_clone<F>(name: &str, iterations: usize, mut f: F) -> BenchResult
where
    F: FnMut(),
{
    // Warmup
    for _ in 0..100 {
        f();
    }

    // Reset allocator
    GLOBAL_TRACKER.reset();

    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    let elapsed = start.elapsed();

    let (alloc_count, alloc_bytes) = GLOBAL_TRACKER.snapshot();

    BenchResult {
        name: name.to_string(),
        iterations,
        total_time: elapsed,
        alloc_count,
        alloc_bytes,
    }
}

fn main() {
    println!("╔═══════════════════════════════════════════════════════════════════════╗");
    println!("║           Arc<str> Optimization Benchmark - BEFORE Optimization       ║");
    println!("╚═══════════════════════════════════════════════════════════════════════╝");
    println!();

    const ITERATIONS: usize = 10_000;
    const LARGE_ITERATIONS: usize = 5_000;

    // Test 1: Message Clone
    println!("═══ Test 1: Message Clone (typical DNS response) ═══");
    let msg = create_test_message();

    let result = bench_clone("Message.clone()", ITERATIONS, || {
        let _cloned = msg.clone();
        std::hint::black_box(&_cloned);
    });
    result.print();

    // Test 2: Arc<Message> Clone
    println!();
    println!("═══ Test 2: Arc<Message> Clone ═══");
    let arc_msg = Arc::new(create_test_message());

    let result = bench_clone("Arc<Message>.clone()", ITERATIONS, || {
        let _cloned = Arc::clone(&arc_msg);
        std::hint::black_box(&_cloned);
    });
    result.print();

    // Test 3: Question Clone
    println!();
    println!("═══ Test 3: Question Clone ═══");
    let question = Question::new(
        "very-long-subdomain.deeply-nested.example.com",
        RecordType::A,
        RecordClass::IN,
    );

    let result = bench_clone("Question.clone()", ITERATIONS, || {
        let _cloned = question.clone();
        std::hint::black_box(&_cloned);
    });
    result.print();

    // Test 4: ResourceRecord Clone
    println!();
    println!("═══ Test 4: ResourceRecord Clone ═══");
    let record = ResourceRecord::new(
        "very-long-subdomain.deeply-nested.example.com",
        RecordType::CNAME,
        RecordClass::IN,
        3600,
        RData::CNAME("another-very-long-domain.example.com".to_string()),
    );

    let result = bench_clone("ResourceRecord.clone()", ITERATIONS, || {
        let _cloned = record.clone();
        std::hint::black_box(&_cloned);
    });
    result.print();

    // Test 5: Large Message Clone
    println!();
    println!("═══ Test 5: Large Message Clone (many records) ═══");
    let large_msg = create_large_message();

    let result = bench_clone("LargeMessage.clone()", LARGE_ITERATIONS, || {
        let _cloned = large_msg.clone();
        std::hint::black_box(&_cloned);
    });
    result.print();

    // Test 6: Concurrent Scenario
    println!();
    println!("═══ Test 6: Concurrent Scenario (16 upstream clones) ═══");
    let msg = create_test_message();
    const UPSTREAM_COUNT: usize = 16;

    let result = bench_clone("16x Message.clone()", ITERATIONS / 10, || {
        for _ in 0..UPSTREAM_COUNT {
            let _cloned = msg.clone();
            std::hint::black_box(&_cloned);
        }
    });
    result.print();

    // Test 7: String Allocation Impact
    println!();
    println!("═══ Test 7: String Allocation Impact ═══");
    let domain = "subdomain.example.com".to_string();

    let result = bench_clone("String.clone()", ITERATIONS * 10, || {
        let _cloned = domain.clone();
        std::hint::black_box(&_cloned);
    });
    result.print();

    // Arc<str> for comparison
    let arc_domain: Arc<str> = Arc::from(domain.as_str());
    let result = bench_clone("Arc<str>.clone()", ITERATIONS * 10, || {
        let _cloned = Arc::clone(&arc_domain);
        std::hint::black_box(&_cloned);
    });
    result.print();

    // Summary
    println!();
    println!("═══════════════════════════════════════════════════════════════════════");
    println!("Summary: Current implementation uses String, which requires heap");
    println!("         allocation on every clone. Arc<str> would reduce this to");
    println!("         a simple atomic increment.");
    println!("═══════════════════════════════════════════════════════════════════════");
}
