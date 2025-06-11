use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct JoinInfo {
    pub token: String,
    pub ip: String,
    pub port: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum NodeMessage {
    RequestConnection(String),  // request to connect to another node
    ConnectionInfo(String, u16),// IP and port for a connection
    GetConnectedNodes,
    ConnectedNodes(Vec<String>),
    Heartbeat,
    AssignTask(String, String), // task id and payload
    TaskResult(String, String), // task id and result
    Disconnect,
}

