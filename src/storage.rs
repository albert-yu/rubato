use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;

#[async_trait]
pub trait StorageService: Send + Sync {
    /// Uploads data to the storage provider and returns the public URL or file path.
    async fn upload(&self, key: &str, data: Vec<u8>, content_type: &str) -> Result<String>;

    /// Deletes a file from the storage provider.
    async fn delete(&self, key: &str) -> Result<()>;
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

#[async_trait]
impl StorageService for LocalStorage {
    async fn upload(&self, key: &str, data: Vec<u8>, _content_type: &str) -> Result<String> {
        let file_path = self.base_path.join(key);
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&file_path, data).await?;

        // Return relative URL (e.g., "/uploads/my-image.jpg")
        Ok(format!("{}/{}", self.base_url, key))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let file_path = self.base_path.join(key);
        if file_path.exists() {
            tokio::fs::remove_file(file_path).await?;
        }
        Ok(())
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
    async fn upload(&self, key: &str, data: Vec<u8>, content_type: &str) -> Result<String> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(data.into())
            .content_type(content_type)
            // .acl(aws_sdk_s3::types::ObjectCannedAcl::PublicRead) // Optional: specific ACLs
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("S3 Upload failed: {}", e))?;

        Ok(format!("{}/{}", self.public_url, key))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("S3 Delete failed: {}", e))?;
        Ok(())
    }
}
