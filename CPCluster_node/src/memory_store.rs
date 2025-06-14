use dashmap::DashMap;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct MemoryStore {
    inner: Arc<DashMap<String, Vec<u8>>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn store(&self, id: String, data: Vec<u8>) {
        self.inner.insert(id, data);
    }

    pub async fn load(&self, id: &str) -> Option<Vec<u8>> {
        self.inner.get(id).map(|v| v.value().clone())
    }

    /// Return a list of all stored keys with their size in bytes
    pub async fn stats(&self) -> Vec<(String, usize)> {
        self.inner
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().len()))
            .collect()
    }
}
