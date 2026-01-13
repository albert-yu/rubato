use anyhow::Result;
use async_trait::async_trait;
use axum::body::Body;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

pub struct FileResponse {
    pub body: Body,
    pub content_type: String,
    pub content_length: u64,
    pub content_range: Option<String>,
    pub accept_ranges: String,
}

#[async_trait]
pub trait StorageService: Send + Sync {
    /// Uploads data to the storage provider and returns the public URL or file path.
    async fn upload(
        &self,
        folder: &str,
        key: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> Result<String>;

    /// Deletes a file from the storage provider.
    async fn delete(&self, folder: &str, key: &str) -> Result<()>;

    /// Returns the public URL for a given key.
    fn get_url(&self, folder: &str, key: &str) -> Result<String>;

    /// Retrieves content, optionally serving a partial range.
    async fn get_content(
        &self,
        folder: &str,
        key: &str,
        range: Option<&str>,
    ) -> Result<FileResponse>;
}

// --- Local Filesystem Implementation ---

#[derive(Clone)]
pub struct LocalStorage {
    base_path: PathBuf,
    base_url: String,
}

impl LocalStorage {
    pub fn new(base_path: &str, base_url: &str) -> Self {
        std::fs::create_dir_all(base_path).expect("Failed to create upload directory");
        Self {
            base_path: PathBuf::from(base_path),
            base_url: base_url.to_string(),
        }
    }
}

fn parse_range_header(range_header: &str, file_size: u64) -> Option<(u64, u64)> {
    if !range_header.starts_with("bytes=") {
        return None;
    }
    let range_str = &range_header[6..];
    let parts: Vec<&str> = range_str.split('-').collect();
    if parts.len() != 2 {
        return None;
    }

    let start_str = parts[0];
    let end_str = parts[1];

    let start = if start_str.is_empty() {
        None
    } else {
        start_str.parse::<u64>().ok()
    };

    let end = if end_str.is_empty() {
        None
    } else {
        end_str.parse::<u64>().ok()
    };

    match (start, end) {
        (Some(s), Some(e)) => {
            if s <= e && s < file_size {
                Some((s, std::cmp::min(e, file_size - 1)))
            } else {
                None
            }
        }
        (Some(s), None) => {
            if s < file_size {
                Some((s, file_size - 1))
            } else {
                None
            }
        }
        (None, Some(e)) => {
            if e == 0 {
                return None;
            }
            let s = if e > file_size { 0 } else { file_size - e };
            Some((s, file_size - 1))
        }
        _ => None,
    }
}

#[async_trait]
impl StorageService for LocalStorage {
    async fn upload(
        &self,
        folder: &str,
        key: &str,
        data: Vec<u8>,
        _content_type: &str,
    ) -> Result<String> {
        if folder.contains("..")
            || folder.contains('\\')
            || key.contains("..")
            || key.contains('\\')
        {
            return Err(anyhow::anyhow!("Not found"));
        }
        let folder = folder.trim_start_matches('/');
        let full_key = format!("{}/{}", folder, key);
        let file_path = self.base_path.join(&full_key);
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&file_path, data).await?;

        // Return relative URL (e.g., "/uploads/my-image.jpg")
        Ok(format!("{}/{}", self.base_url, full_key))
    }

    async fn delete(&self, folder: &str, key: &str) -> Result<()> {
        if folder.contains("..")
            || folder.contains('\\')
            || key.contains("..")
            || key.contains('\\')
        {
            return Err(anyhow::anyhow!("Not found"));
        }
        let folder = folder.trim_start_matches('/');
        let file_path = self.base_path.join(folder).join(key);
        if file_path.exists() {
            tokio::fs::remove_file(file_path).await?;
        }
        Ok(())
    }

    fn get_url(&self, folder: &str, key: &str) -> Result<String> {
        if folder.contains("..")
            || folder.contains('\\')
            || key.contains("..")
            || key.contains('\\')
        {
            return Err(anyhow::anyhow!("Not found"));
        }
        let folder = folder.trim_start_matches('/');
        Ok(format!("{}/{}/{}", self.base_url, folder, key))
    }

    async fn get_content(
        &self,
        folder: &str,
        key: &str,
        range: Option<&str>,
    ) -> Result<FileResponse> {
        if folder.contains("..")
            || folder.contains('\\')
            || key.contains("..")
            || key.contains('\\')
        {
            return Err(anyhow::anyhow!("Not found"));
        }
        let folder = folder.trim_start_matches('/');
        let file_path = self.base_path.join(folder).join(key);

        let mut file = tokio::fs::File::open(&file_path).await?;
        let metadata = file.metadata().await?;
        let file_size = metadata.len();

        // Infer content type from the beginning of the file
        let mut head = [0u8; 1024];
        let n = file.read(&mut head).await?;
        let content_type = infer::get(&head[..n])
            .map(|k| k.mime_type())
            .unwrap_or("application/octet-stream")
            .to_string();

        let (start, end) = if let Some(range_header) = range {
            parse_range_header(range_header, file_size).unwrap_or((0, file_size - 1))
        } else {
            (0, file_size - 1)
        };

        let length = end - start + 1;
        file.seek(std::io::SeekFrom::Start(start)).await?;

        let stream = ReaderStream::new(file.take(length));
        let body = Body::from_stream(stream);

        let content_range = if range.is_some() {
            Some(format!("bytes {}-{}/{}", start, end, file_size))
        } else {
            None
        };

        Ok(FileResponse {
            body,
            content_type,
            content_length: length,
            content_range,
            accept_ranges: "bytes".to_string(),
        })
    }
}

// --- S3 Implementation ---

#[derive(Clone)]
pub struct S3Storage {
    client: aws_sdk_s3::Client,
    bucket: String,
    public_url: String, // e.g., "https://my-bucket.s3.amazonaws.com"
}

impl S3Storage {
    pub fn new(client: aws_sdk_s3::Client, bucket: String, public_url: String) -> Self {
        Self {
            client,
            bucket,
            public_url,
        }
    }
}

#[async_trait]
impl StorageService for S3Storage {
    async fn upload(
        &self,
        folder: &str,
        key: &str,
        data: Vec<u8>,
        content_type: &str,
    ) -> Result<String> {
        let folder = folder.trim_start_matches('/');
        let full_key = format!("{}/{}", folder, key);
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .body(data.into())
            .content_type(content_type)
            // .acl(aws_sdk_s3::types::ObjectCannedAcl::PublicRead) // Optional: specific ACLs
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("S3 Upload failed: {}", e))?;

        Ok(format!("{}/{}", self.public_url, full_key))
    }

    async fn delete(&self, folder: &str, key: &str) -> Result<()> {
        let folder = folder.trim_start_matches('/');
        let full_key = format!("{}/{}", folder, key);
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(full_key)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("S3 Delete failed: {}", e))?;
        Ok(())
    }

    fn get_url(&self, folder: &str, key: &str) -> Result<String> {
        let folder = folder.trim_start_matches('/');
        Ok(format!("{}/{}/{}", self.public_url, folder, key))
    }

    async fn get_content(
        &self,
        folder: &str,
        key: &str,
        range: Option<&str>,
    ) -> Result<FileResponse> {
        let folder = folder.trim_start_matches('/');
        let full_key = format!("{}/{}", folder, key);
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&full_key)
            .set_range(range.map(|s| s.to_string()))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("S3 GetObject failed: {}", e))?;

        let content_type = output
            .content_type
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let content_range = output.content_range;
        let content_length = output.content_length.unwrap_or(0) as u64;
        let body = Body::from_stream(ReaderStream::new(output.body.into_async_read()));

        Ok(FileResponse {
            body,
            content_type,
            content_length,
            content_range,
            accept_ranges: "bytes".to_string(),
        })
    }
}
