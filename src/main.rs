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
use std::{collections::HashMap, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{broadcast, mpsc, oneshot},
    time,
};
use tower_http::services::{ServeDir, ServeFile};

type PlayerId = u64;

enum Cmd {
    Join {
        client_tx: mpsc::UnboundedSender<Message>,
        reply: oneshot::Sender<PlayerId>,
    },
    Leave {
        id: PlayerId,
    },
    Input {
        id: PlayerId,
        text: String,
    },
}

#[derive(Clone)]
struct AppState {
    cmd_tx: mpsc::UnboundedSender<Cmd>,
}

struct Player {
    id: PlayerId,
    tx: mpsc::UnboundedSender<Message>,
    // TODO: pos, vel, radius, etc..
}

#[tokio::main]
async fn main() -> Result<()> {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Cmd>();

    tokio::spawn(game_loop(cmd_rx));

    let state = AppState { cmd_tx };
    // set up routes with state
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

    let (mut client_tx, mut client_rx) = mpsc::unbounded_channel::<Message>();

    let (reply_tx, reply_rx) = oneshot::channel::<PlayerId>();

    if state
        .cmd_tx
        .send(Cmd::Join {
            client_tx: client_tx.clone(),
            reply: reply_tx,
        })
        .is_err()
    {
        return;
    }

    let id = match reply_rx.await {
        Ok(id) => id,
        Err(_) => return,
    };

    let write_task = tokio::spawn(async move {
        while let Some(msg) = client_rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Text(t) => {
                let _ = state.cmd_tx.send(Cmd::Input {
                    id,
                    text: t.to_string(),
                });
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // Inform the game loop that this player left
    let _ = state.cmd_tx.send(Cmd::Leave { id });

    write_task.abort();
}

async fn game_loop(mut cmd_rx: mpsc::UnboundedReceiver<Cmd>) {
    let mut tick = time::interval(Duration::from_millis(50)); // 20 TPS for now

    let mut next_id: PlayerId = 1;
    let mut players: HashMap<PlayerId, Player> = HashMap::new();
    let mut counter: u64 = 0;

    loop {
        tokio::select! {
            _ = tick.tick() => {
                counter += 1;

                // TODO: step physics, collisions, rebuild spatial index, etc.

                let mut dead = Vec::new();
                for (id, p) in players.iter() {
                    let payload = format!(r#"{{"type":"tick","n":{},"you":{}}}"#, counter, id);
                    if p.tx.send(Message::Text(payload.into())).is_err() {
                        dead.push(*id);
                    }
                }
                for id in dead {
                    players.remove(&id);
                }
            }

            Some(cmd) = cmd_rx.recv() => {
                match cmd {
                    Cmd::Join { client_tx, reply } => {
                        let id = next_id;
                        next_id += 1;

                        players.insert(id, Player { id, tx: client_tx });

                        let _ = reply.send(id);

                        if let Some(p) = players.get(&id) {
                            let _ = p.tx.send(Message::Text(format!(r#"{{"type":"welcome","id":{}}}"#, id).into()));
                        }
                    }

                    Cmd::Leave { id } => {
                        players.remove(&id);
                    }

                    Cmd::Input { id, text } => {
                        // TODO: parse/store input on that player's state (aim, split, etc)
                        println!("input from {id}: {text}");
                    }
                }
            }
        }
    }
}
