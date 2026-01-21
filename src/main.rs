use anyhow::Result;
use axum::{
    Router,
    extract::{
        WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
    routing::{any, get},
};
use futures_util::{
    SinkExt,
    stream::{SplitSink, SplitStream, StreamExt},
};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

#[tokio::main]
async fn main() -> Result<()> {
    let app: axum::Router = Router::new()
        .route("/ws", any(ws_handler))
        .route_service("/", ServeFile::new("static/index.html"))
        .nest_service("/static", ServeDir::new("static"));

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();

    axum::serve(listener, app).await.unwrap();

    Ok(())
}

async fn ws_handler(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(socket: WebSocket) {
    let (sender, receiver) = socket.split();

    tokio::spawn(write(sender));
    tokio::spawn(read(receiver));
}

async fn read(mut receiver: SplitStream<WebSocket>) {
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(t) => {
                println!("client says: {t}");
            }
            Message::Binary(b) => {
                println!("client sent {} bytes", b.len());
            }
            Message::Close(_) => {
                println!("client closed");
                break;
            }
            _ => {}
        }
    }
}

async fn write(mut sender: SplitSink<WebSocket, Message>) {
    // For now, just send a welcome message once.
    let _ = sender
        .send(Message::Text("Welcome from server".into()))
        .await;
}
