use anyhow::Result;
use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
    routing::any,
};
use futures_util::{SinkExt, stream::StreamExt};
use std::time::Duration;
use tokio::{
    net::TcpListener,
    sync::{broadcast, mpsc},
    time,
};
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
struct AppState {
    // server -> clients
    updates: broadcast::Sender<String>,
    // clients -> server
    inputs: mpsc::UnboundedSender<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let (updates_tx, _) = broadcast::channel::<String>(128);
    let (inputs_tx, inputs_rx) = mpsc::unbounded_channel::<String>();

    tokio::spawn(game_loop(updates_tx.clone(), inputs_rx));

    let state = AppState {
        updates: updates_tx,
        inputs: inputs_tx,
    };

    let app: axum::Router = Router::new()
        .route("/ws", any(ws_handler))
        .route_service("/", ServeFile::new("static/index.html"))
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();

    axum::serve(listener, app).await.unwrap();

    Ok(())
}

async fn ws_handler(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    let mut updates_rx = state.updates.subscribe();

    let write_task = tokio::spawn(async move {
        loop {
            match updates_rx.recv().await {
                Ok(s) => {
                    if ws_tx.send(Message::Text(s.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // client fell behind so we skip old updates
                    continue;
                }
                Err(_) => break,
            }
        }
    });

    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Text(t) => {
                let _ = state.inputs.send(t.to_string());
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    write_task.abort();
}

async fn game_loop(
    updates: broadcast::Sender<String>,
    mut inputs_rx: mpsc::UnboundedReceiver<String>,
) {
    let mut tick = time::interval(Duration::from_millis(50)); // 20 TPS for now
    let mut counter: u64 = 0;

    loop {
        tokio::select! {
            _ = tick.tick() => {
                counter += 1;

                // TODO: step physics, collisions, rebuild quadtree, etc.
                let state_json = format!(r#"{{"type":"tick","n":{}}}"#, counter);

                let _ = updates.send(state_json);
            }

            Some(input) = inputs_rx.recv() => {
                // TODO: parse input, store per-player aim, etc.
                println!("input: {input}");
            }
        }
    }
}
