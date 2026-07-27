//! WebSocket handler for real-time metrics streaming

use crate::web::state::WebState;
use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tracing::{debug, error, info, trace, warn};

/// WebSocket message types from client
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Subscribe to metrics updates
    Subscribe { topics: Vec<String> },
    /// Unsubscribe from topics
    Unsubscribe { topics: Vec<String> },
    /// Ping message for keepalive
    Ping,
}

/// WebSocket message types to client
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Connection established
    Connected { version: String },
    /// Pong response
    Pong,
    /// Metrics update
    Metrics { data: MetricsUpdate },
    /// Error message
    Error { message: String },
}

/// Metrics update payload
#[derive(Debug, Clone, Serialize)]
pub struct MetricsUpdate {
    pub timestamp: u64,
    pub qps: f64,
    pub total_queries: u64,
    pub cache_hit_rate: f64,
    pub error_rate: f64,
}

/// GET /ws/metrics - WebSocket endpoint for real-time metrics
pub async fn metrics_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<WebState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

/// Handle a WebSocket connection
async fn handle_socket(socket: WebSocket, state: Arc<WebState>) {
    let (mut sender, mut receiver) = socket.split();

    let heartbeat_secs = state.config().websocket.heartbeat_secs;
    let timeout_secs = state.config().websocket.timeout_secs;
    let ws_counter = state.ws_connections();

    ws_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    info!("New WebSocket connection for metrics");

    // Send connected message
    let connected_msg = ServerMessage::Connected {
        version: "1.0".to_string(),
    };
    if let Err(e) = send_message(&mut sender, &connected_msg).await {
        error!(error = %e, "Failed to send connected message");
        ws_counter.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    // Subscribed topics
    let mut subscribed_topics: Vec<String> = vec!["overview".to_string()];

    // Create intervals for sending updates and heartbeat
    let mut update_interval = interval(Duration::from_secs(1));
    let mut heartbeat_interval = interval(Duration::from_secs(heartbeat_secs));

    // Last activity timestamp for timeout detection
    let mut last_pong = std::time::Instant::now();

    loop {
        tokio::select! {
            // Handle incoming messages
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(client_msg) => {
                                match client_msg {
                                    ClientMessage::Subscribe { topics } => {
                                        debug!(topics = ?topics, "Client subscribed to topics");
                                        for topic in topics {
                                            if !subscribed_topics.contains(&topic) {
                                                subscribed_topics.push(topic);
                                            }
                                        }
                                    }
                                    ClientMessage::Unsubscribe { topics } => {
                                        debug!(topics = ?topics, "Client unsubscribed from topics");
                                        subscribed_topics.retain(|t| !topics.contains(t));
                                    }
                                    ClientMessage::Ping => {
                                        trace!("Received ping");
                                        last_pong = std::time::Instant::now();
                                        let _ = send_message(&mut sender, &ServerMessage::Pong).await;
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to parse client message");
                                let _ = send_message(&mut sender, &ServerMessage::Error {
                                    message: "Invalid message format".to_string(),
                                }).await;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if let Err(e) = sender.send(Message::Pong(data)).await {
                            error!(error = %e, "Failed to send pong");
                            break;
                        }
                        last_pong = std::time::Instant::now();
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_pong = std::time::Instant::now();
                    }
                    Some(Ok(Message::Close(_))) => {
                        debug!("Client closed connection");
                        break;
                    }
                    Some(Err(e)) => {
                        error!(error = %e, "WebSocket error");
                        break;
                    }
                    None => {
                        debug!("WebSocket stream ended");
                        break;
                    }
                    _ => {}
                }
            }

            // Send periodic updates
            _ = update_interval.tick() => {
                if subscribed_topics.contains(&"overview".to_string()) {
                    let overview = state.metrics_collector().get_overview();
                    let update = MetricsUpdate {
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        qps: overview.queries_per_second,
                        total_queries: overview.total_queries,
                        cache_hit_rate: overview.cache_hit_rate,
                        error_rate: if overview.total_queries > 0 {
                            (overview.error_responses as f64 / overview.total_queries as f64) * 100.0
                        } else {
                            0.0
                        },
                    };

                    if let Err(e) = send_message(&mut sender, &ServerMessage::Metrics { data: update }).await {
                        error!(error = %e, "Failed to send metrics update");
                        break;
                    }
                }
            }

            // Heartbeat check
            _ = heartbeat_interval.tick() => {
                // Check if client has responded recently
                if last_pong.elapsed() > Duration::from_secs(timeout_secs) {
                    warn!("WebSocket client timed out");
                    break;
                }

                // Send ping
                if let Err(e) = sender.send(Message::Ping(vec![].into())).await {
                    error!(error = %e, "Failed to send ping");
                    break;
                }
            }
        }
    }

    ws_counter.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    info!("WebSocket connection closed");
}

/// Send a JSON message over WebSocket
async fn send_message(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    msg: &ServerMessage,
) -> Result<(), axum::Error> {
    match serde_json::to_string(msg) {
        Ok(json) => sender.send(Message::Text(json.into())).await,
        Err(e) => {
            error!(error = %e, "Failed to serialize message");
            Ok(())
        }
    }
}
