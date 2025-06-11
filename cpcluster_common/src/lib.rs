use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct JoinInfo {
    pub token: String,
    pub ip: String,
    pub port: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum NodeMessage {
    RequestConnection(String),
    ConnectionInfo(String, u16),
    GetConnectedNodes,
    ConnectedNodes(Vec<String>),
    Disconnect,
}
