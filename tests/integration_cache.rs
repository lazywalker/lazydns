use async_trait::async_trait;
use lazydns::Result;
use lazydns::dns::{Message, Question, RData, RecordClass, RecordType, ResourceRecord};
use lazydns::plugin::{Context, Plugin, PluginHandler, Registry};
use lazydns::plugins::cache::CachePlugin;
use std::net::Ipv4Addr;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::time::{Duration, Instant, sleep};

const STALE_RESPONSE_TTL: u32 = 5;

#[derive(Debug)]
struct TestResponder {
    ttl: u32,
    call_count: Arc<AtomicUsize>,
}

impl TestResponder {
    fn new(ttl: u32, call_count: Arc<AtomicUsize>) -> Self {
        Self { ttl, call_count }
    }
}

#[async_trait]
impl Plugin for TestResponder {
    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        self.call_count.fetch_add(1, Ordering::SeqCst);

        let question = ctx
            .request()
            .questions()
            .first()
            .expect("request must contain a question");

        let mut response = Message::new();
        response.set_response(true);
        response.set_id(ctx.request().id());
        response.add_question(question.clone());
        response.add_answer(ResourceRecord::new(
            question.qname(),
            question.qtype(),
            question.qclass(),
            self.ttl,
            RData::A(Ipv4Addr::new(203, 0, 113, 1)),
        ));

        ctx.set_response(Some(response));
        Ok(())
    }

    fn name(&self) -> &str {
        "test_responder"
    }
}

fn make_request() -> Message {
    let mut request = Message::new();
    request.set_id(4321);
    request.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
    request
}

fn build_handler(resolver: &Arc<TestResponder>) -> Arc<PluginHandler> {
    let mut registry = Registry::new();
    let resolver_plugin: Arc<dyn Plugin> = resolver.clone();
    registry
        .register(resolver_plugin)
        .expect("register resolver");
    Arc::new(PluginHandler {
        registry: Arc::new(registry),
        entry: resolver.name().to_string(),
    })
}

fn prepare_context(request: &Message, handler: &Arc<PluginHandler>) -> Context {
    let mut ctx = Context::new(request.clone());
    ctx.set_metadata("lazy_refresh_handler", Arc::clone(handler));
    ctx.set_metadata("lazy_refresh_entry", handler.entry.clone());
    ctx
}

async fn wait_for_call_count(counter: &AtomicUsize, target: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while counter.load(Ordering::SeqCst) < target {
        if Instant::now() >= deadline {
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(
        counter.load(Ordering::SeqCst) >= target,
        "background refresh did not run in time"
    );
}

#[tokio::test]
// fills an entry, waits until the remaining TTL hits the configured threshold,
// and asserts that a stale hit still returns an answer
// while the resolver’s call count goes from 1→2
// thanks to the background refresh.
async fn integration_cache_lazycache_refresh_triggers_background() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let resolver = Arc::new(TestResponder::new(2, Arc::clone(&call_count)));
    let handler = build_handler(&resolver);
    let cache = CachePlugin::new(32).with_lazycache(0.5);
    let request = make_request();

    let mut populate_ctx = prepare_context(&request, &handler);
    cache
        .execute(&mut populate_ctx)
        .await
        .expect("cache phase 1");
    resolver
        .execute(&mut populate_ctx)
        .await
        .expect("resolver produced a response");
    cache
        .execute(&mut populate_ctx)
        .await
        .expect("cache stored response");

    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    sleep(Duration::from_millis(1_200)).await;

    let mut query_ctx = prepare_context(&request, &handler);
    cache.execute(&mut query_ctx).await.expect("cache lazy hit");

    let response = query_ctx
        .response()
        .expect("stale response should be returned");
    assert_eq!(response.answer_count(), 1);

    wait_for_call_count(&call_count, 2).await;
}

#[tokio::test]
// lets a cached entry expire but keeps it alive via cache_ttl,
// checks that the response TTL is reset to the 5s stale TTL,
// and again waits for the resolver to run in the background.
async fn integration_cache_ttl_triggers_stale_serving() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let resolver = Arc::new(TestResponder::new(1, Arc::clone(&call_count)));
    let handler = build_handler(&resolver);
    let cache = CachePlugin::new(32).with_cache_ttl(10);
    let request = make_request();

    let mut populate_ctx = prepare_context(&request, &handler);
    cache
        .execute(&mut populate_ctx)
        .await
        .expect("cache phase 1");
    resolver
        .execute(&mut populate_ctx)
        .await
        .expect("resolver produced a response");
    cache
        .execute(&mut populate_ctx)
        .await
        .expect("cache stored response");

    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    sleep(Duration::from_millis(1_200)).await;

    let mut stale_ctx = prepare_context(&request, &handler);
    cache
        .execute(&mut stale_ctx)
        .await
        .expect("stale-serving cache hit");

    let response = stale_ctx
        .response()
        .expect("stale response should be available");
    assert_eq!(response.answers()[0].ttl(), STALE_RESPONSE_TTL);

    wait_for_call_count(&call_count, 2).await;
}

/// Regression test for the `refreshing_keys` leak.
///
/// Before the fix, the cache's `refreshing_keys` dedup set was only cleared
/// on the enqueue-failure paths. After the first successful background
/// refresh, the key stayed in the set forever, so every subsequent lazy hit
/// was skipped with "already being refreshed" and LazyCache prefetch silently
/// died after the first success.
///
/// This test triggers one background refresh and asserts the key is removed
/// from `refreshing_keys` once the refresh completes (via the test-only
/// `is_refreshing` accessor). With the leak, the key would remain forever.
#[tokio::test]
async fn integration_lazycache_refresh_key_is_cleared_after_completion() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let resolver = Arc::new(TestResponder::new(2, Arc::clone(&call_count)));
    let handler = build_handler(&resolver);
    let cache = CachePlugin::new(32).with_lazycache(0.5);
    let request = make_request();

    // Populate the cache (resolver call #1).
    let mut populate_ctx = prepare_context(&request, &handler);
    cache
        .execute(&mut populate_ctx)
        .await
        .expect("cache phase 1");
    resolver
        .execute(&mut populate_ctx)
        .await
        .expect("resolver produced a response");
    cache
        .execute(&mut populate_ctx)
        .await
        .expect("cache stored response");
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // Wait until the remaining TTL drops below the 50% threshold, then hit
    // the cache to trigger a background refresh (resolver call #2).
    sleep(Duration::from_millis(1_200)).await;
    let mut lazy_hit = prepare_context(&request, &handler);
    cache
        .execute(&mut lazy_hit)
        .await
        .expect("lazy hit triggers refresh");
    wait_for_call_count(&call_count, 2).await;

    // Cache key used by make_request ("example.com", A, IN).
    let key = "example.com:1:1";

    // The completion callback runs after the worker finishes (regardless of
    // outcome). Give it a short grace period past call_count==2, then assert
    // the key has been removed from the dedup set.
    let deadline = Instant::now() + Duration::from_secs(2);
    while cache.is_refreshing(key) && Instant::now() < deadline {
        sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !cache.is_refreshing(key),
        "refreshing_keys was not cleared after background refresh completed \
         (leak regression): the key would block all future LazyCache refreshes"
    );
}
