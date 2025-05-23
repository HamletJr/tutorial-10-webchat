use futures_util::sink::SinkExt;
use futures_util::stream::StreamExt;
use std::error::Error;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::{channel, Sender};
use tokio_websockets::{Message, ServerBuilder, WebSocketStream};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MessageData {
    pub message_type: String,
    pub data_array: Option<Vec<String>>,
    pub data: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub from: String,
    pub message: String,
}

pub struct User {
    websocket_addr: SocketAddr,
    nickname: String,
    is_online: bool,
}

pub struct UserRepository {
    pub users: Arc<Mutex<Vec<User>>>,
}

async fn handle_connection(
    addr: SocketAddr,
    mut ws_stream: WebSocketStream<TcpStream>,
    bcast_tx: Sender<String>,
    user_repo: Arc<Mutex<Vec<User>>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {

    let mut bcast_rx = bcast_tx.subscribe();

    // A continuous loop for concurrently performing two tasks: (1) receiving
    // messages from `ws_stream` and broadcasting them, and (2) receiving
    // messages on `bcast_rx` and sending them to the client.
    loop {
        tokio::select! {
            incoming = ws_stream.next() => {
                match incoming {
                    Some(Ok(msg)) => {
                        let msg_data: MessageData = serde_json::from_str(msg.as_text().unwrap_or_default())
                            .unwrap();
                        match msg_data.message_type.as_str() {
                            "register" => {
                                user_repo.lock().unwrap().push(User {
                                    websocket_addr: addr,
                                    nickname: msg_data.data.unwrap_or_default(),
                                    is_online: true,
                                });
                                let message = MessageData {
                                    message_type: String::from("users"),
                                    data_array: Some(user_repo.lock().unwrap().iter()
                                        .map(|user| user.nickname.clone())
                                        .collect()),
                                    data: None,
                                };
                                let json = serde_json::to_string(&message)?;
                                println!("Broadcasting to all clients: {json:?}");
                                bcast_tx.send(json)?;
                            }
                            "message" => {
                                let users = user_repo.lock().unwrap();
                                if let Some(sender) = users.iter().find(|user| user.websocket_addr == addr) {
                                    let message = MessageData {
                                        message_type: String::from("message"),
                                        data_array: None,
                                        data: Some(serde_json::to_string(&ChatMessage {
                                            from: sender.nickname.clone(),
                                            message: msg_data.data.unwrap_or_default(),
                                        })?),
                                    };
                                    let json = serde_json::to_string(&message)?;
                                    println!("Broadcasting to all clients: {json:?}");
                                    bcast_tx.send(json)?;
                                }
                            }
                            _ => {
                                println!("Unknown message type from client {addr:?}: {msg:?}");
                            }
                        }
                        if let Some(text) = msg.as_text() {
                            println!("From client {addr:?} {text:?}");
                        }
                    }
                    Some(Err(err)) => return Err(err.into()),
                    None => return Ok(()),
                }
            }
            msg = bcast_rx.recv() => {
                ws_stream.send(Message::text(msg?)).await?;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let (bcast_tx, _) = channel(16);
    let user_repo = UserRepository {
        users: Arc::new(Mutex::new(Vec::new())),
    };

    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    println!("listening on port 8080");

    loop {
        let (socket, addr) = listener.accept().await?;
        println!("New connection from {addr:?}");
        let bcast_tx = bcast_tx.clone();
        let user_repo = Arc::clone(&user_repo.users);
        tokio::spawn(async move {
            // Wrap the raw TCP stream into a websocket.
            let (_req, ws_stream) = ServerBuilder::new().accept(socket).await?;

            handle_connection(addr, ws_stream, bcast_tx, user_repo).await
        });
    }
}