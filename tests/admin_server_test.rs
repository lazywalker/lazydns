//! Integration test for Admin Server

#[cfg(all(test, feature = "admin"))]
mod admin_server_integration_tests {
    use lazydns::config::Config;
    use lazydns::server::admin::{AdminServer, AdminState};
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tokio::time::{Duration, sleep};

    #[tokio::test]
    async fn test_admin_server_startup() {
        let config = Arc::new(RwLock::new(Config::new()));
        let registry = Arc::new(lazydns::plugin::Registry::new());
        let state = AdminState::new(Arc::clone(&config), Arc::clone(&registry));
        let server = AdminServer::new("127.0.0.1:9999", state);

        // Spawn server in background
        let server_task = tokio::spawn(async move { server.run().await });

        // Give server time to start
        sleep(Duration::from_millis(500)).await;

        // Test if server is responding
        let client = reqwest::Client::new();
        let response = client
            .get("http://127.0.0.1:9999/api/server/stats")
            .send()
            .await;

        // Abort server task
        server_task.abort();

        // Verify response
        assert!(response.is_ok(), "Server should be responding");
        let resp = response.unwrap();
        assert_eq!(resp.status(), 200);

        let body = resp.json::<serde_json::Value>().await.unwrap();
        assert_eq!(body["status"], "running");
        assert!(body["version"].is_string());
    }
}
