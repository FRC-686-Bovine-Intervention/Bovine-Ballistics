use std::{fs, net::SocketAddr};
use axum::{
    extract::{Extension, Json},
    response::{IntoResponse, Redirect, Response},
    routing::{get, put},
    http::{StatusCode, header},
    Router,
};
use axum::body::Body;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{Message, WebSocket};
use tower_http::services::ServeDir;
use tokio::sync::broadcast;
use serde_json::Value;

pub struct WebInterface {
    port: u16,
    tx: broadcast::Sender<String>,
}

impl WebInterface {
    pub fn new(port: u16) -> Self {
        let (tx, _) = broadcast::channel(32);
        Self { port, tx }
    }

    pub async fn run(&self) {
        let tx_clone = self.tx.clone();

        // handler to serve static HTML/JS/CSS
        let static_files = ServeDir::new("static");

        let app = Router::new()
            .route("/", get(|| async { Redirect::permanent("/index.html") }))
            .route("/ws", get(handle_ws))

            // === NEW: GET /settings returns the raw JSON file as text/plain ===
            .route("/settings", get(|| async {
                match fs::read_to_string("data/settings.json") {
                    Ok(txt) => Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/plain")
                        .body(Body::from(txt))
                        .unwrap(),
                    Err(_) => Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(Body::empty())
                        .unwrap(),
                }
            }))

            // === NEW: PUT /settings accepts JSON body, writes file, broadcasts update ===
            .route(
                "/settings",
                put(
                    move |Extension(tx): Extension<broadcast::Sender<String>>, Json(payload): Json<Value>| async move {
                        let txt = serde_json::to_string_pretty(&payload).unwrap_or_default();
                        match fs::write("data/settings.json", &txt) {
                            Ok(_) => {
                                let _ = tx.send(r#"{"type":"settings_update"}"#.to_string());
                                StatusCode::OK
                            }
                            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
                        }
                    },
                ),
            )

            // mount static dir for everything else
            .fallback_service(static_files)
            .layer(Extension(tx_clone));

        let addr = SocketAddr::from(([0, 0, 0, 0], self.port));
        eprintln!("Listening on http://{}", addr);
        axum::serve(tokio::net::TcpListener::bind(addr).await.unwrap(), app.into_make_service())
            .await
            .unwrap();
    }
}

async fn handle_ws(
    ws: WebSocketUpgrade,
    Extension(tx): Extension<broadcast::Sender<String>>,
) -> impl IntoResponse {
    let rx = tx.subscribe();
    ws.on_upgrade(move |socket| handle_socket(socket, rx, tx))
}

async fn handle_socket(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<String>,
    _tx: broadcast::Sender<String>,
) {
    loop {
        tokio::select! {
            // Send broadcast messages to this client
            Ok(msg) = rx.recv() => {
                eprintln!("Received message: {:?}", msg);
                if socket.send(msg.into()).await.is_err() {
                    break; // Client disconnected
                }
            }

            // Receive from client (optional, but useful)
            Some(result) = socket.recv() => {
                match result {
                    Ok(Message::Text(txt)) => {
                        eprintln!("Client says: {}", txt);
                        // You could handle client messages here, or forward to `tx.send()` if needed.
                    }
                    Ok(Message::Close(_)) => break,
                    Err(_) => break,
                    _ => {}
                }
            }
        }
    }
}