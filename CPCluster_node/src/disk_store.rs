use std::path::PathBuf;
use tokio::fs;

#[derive(Clone)]
pub struct DiskStore {
    dir: PathBuf,
    quota_bytes: u64,
}

impl DiskStore {
    pub fn new(dir: PathBuf, quota_mb: u64) -> Self {
        Self {
            dir,
            quota_bytes: quota_mb * 1024 * 1024,
        }
    }

    pub async fn store(&self, id: String, data: Vec<u8>) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir).await?;
        let usage = directory_size_async(self.dir.clone()).await?;
        if usage + data.len() as u64 > self.quota_bytes {
            return Err(std::io::Error::other("quota exceeded"));
        }
        fs::write(self.dir.join(id), data).await
    }

    pub async fn load(&self, id: &str) -> Option<Vec<u8>> {
        fs::read(self.dir.join(id)).await.ok()
    }

    /// Return a list of all stored files with their size in bytes and the remaining free space
    pub async fn stats(&self) -> std::io::Result<(Vec<(String, u64)>, u64)> {
        fs::create_dir_all(&self.dir).await?;
        let mut entries = Vec::new();
        let mut dir = fs::read_dir(&self.dir).await?;
        while let Some(entry) = dir.next_entry().await? {
            let meta = entry.metadata().await?;
            if meta.is_file() {
                entries.push((entry.file_name().to_string_lossy().to_string(), meta.len()));
            }
        }
        let used = directory_size_async(self.dir.clone()).await?;
        let free = self.quota_bytes.saturating_sub(used);
        Ok((entries, free))
    }
}

async fn directory_size_async(path: PathBuf) -> std::io::Result<u64> {
    if !fs::try_exists(&path).await? {
        return Ok(0);
    }
    let mut size = 0u64;
    let mut stack = vec![path];
    while let Some(dir_path) = stack.pop() {
        let mut dir = fs::read_dir(dir_path).await?;
        while let Some(entry) = dir.next_entry().await? {
            let meta = entry.metadata().await?;
            if meta.is_file() {
                size += meta.len();
            } else if meta.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    Ok(size)
}
