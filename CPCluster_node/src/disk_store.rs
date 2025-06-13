use std::path::{Path, PathBuf};
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
        let usage = directory_size(&self.dir)?;
        if usage + data.len() as u64 > self.quota_bytes {
            return Err(std::io::Error::other("quota exceeded"));
        }
        fs::write(self.dir.join(id), data).await
    }

    pub async fn load(&self, id: &str) -> Option<Vec<u8>> {
        fs::read(self.dir.join(id)).await.ok()
    }
}

fn directory_size(path: &Path) -> std::io::Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut size = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            size += meta.len();
        } else if meta.is_dir() {
            size += directory_size(&entry.path())?;
        }
    }
    Ok(size)
}
