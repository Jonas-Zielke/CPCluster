use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub struct MemoryStore {
    inner: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn store(&self, id: String, data: Vec<u8>) {
        self.inner.lock().await.insert(id, data);
    }

    pub async fn load(&self, id: &str) -> Option<Vec<u8>> {
        self.inner.lock().await.get(id).cloned()
    }

    /// Return a list of all stored keys with their size in bytes
    pub async fn stats(&self) -> Vec<(String, usize)> {
        let map = self.inner.lock().await;
        map.iter().map(|(k, v)| (k.clone(), v.len())).collect()
    }
}
