use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs;

#[derive(Clone)]
pub struct DiskStore {
    dir: PathBuf,
    quota_bytes: u64,
    usage: Arc<AtomicU64>,
}

impl DiskStore {
    pub fn new(dir: PathBuf, quota_mb: u64) -> Self {
        std::fs::create_dir_all(&dir).ok();
        let usage = directory_size_sync(&dir).unwrap_or(0);
        Self {
            dir,
            quota_bytes: quota_mb * 1024 * 1024,
            usage: Arc::new(AtomicU64::new(usage)),
        }
    }

    pub async fn store(&self, id: String, data: Vec<u8>) -> std::io::Result<()> {
        let path = self.dir.join(&id);
        let old_size = fs::metadata(&path).await.ok().map(|m| m.len()).unwrap_or(0);
        let data_len = data.len() as u64;
        let new_usage = self.usage.load(Ordering::SeqCst).saturating_sub(old_size) + data_len;
        if new_usage > self.quota_bytes {
            return Err(std::io::Error::other("quota exceeded"));
        }
        fs::write(&path, &data).await?;
        self.usage
            .fetch_add(data_len.saturating_sub(old_size), Ordering::SeqCst);
        Ok(())
    }

    pub async fn load(&self, id: &str) -> Option<Vec<u8>> {
        fs::read(self.dir.join(id)).await.ok()
    }

    /// Return a list of all stored files with their size in bytes and the remaining free space
    pub async fn stats(&self) -> std::io::Result<(Vec<(String, u64)>, u64)> {
        let mut entries = Vec::new();
        let mut dir = fs::read_dir(&self.dir).await?;
        while let Some(entry) = dir.next_entry().await? {
            let meta = entry.metadata().await?;
            if meta.is_file() {
                entries.push((entry.file_name().to_string_lossy().to_string(), meta.len()));
            }
        }
        let used = self.usage.load(Ordering::SeqCst);
        let free = self.quota_bytes.saturating_sub(used);
        Ok((entries, free))
    }
}

fn directory_size_sync(path: &Path) -> std::io::Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut size = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir_path) = stack.pop() {
        for entry in std::fs::read_dir(&dir_path)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_file() {
                size += meta.len();
            } else if meta.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    Ok(size)
}
