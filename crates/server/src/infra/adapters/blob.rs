//! Hexagonal BlobStorePort and adapters for file storage and artifact blobs.

use std::path::PathBuf;

use tokio::fs;

use crate::infra::{error::ServerError, state::BoxFuture};

/// Abstract port for artifact and file blob persistence.
pub trait BlobStorePort: Send + Sync + 'static {
    fn put_blob<'a>(
        &'a self,
        key: &'a str,
        data: Vec<u8>,
        content_type: &'a str,
    ) -> BoxFuture<'a, Result<String, ServerError>>;

    fn get_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Vec<u8>, ServerError>>;

    fn delete_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ServerError>>;

    fn presign_url<'a>(
        &'a self,
        key: &'a str,
        expires_secs: u64,
    ) -> BoxFuture<'a, Result<String, ServerError>>;
}

/// Local filesystem blob store for sovereign self-hosted mode and hermetic tests.
#[derive(Clone)]
pub struct LocalFsBlobStore {
    root_dir: PathBuf,
}

impl LocalFsBlobStore {
    #[must_use]
    pub fn new(root_dir: PathBuf) -> Self {
        Self { root_dir }
    }

    fn resolve_path(&self, key: &str) -> Result<PathBuf, ServerError> {
        let clean = key.trim_start_matches('/');
        if clean.contains("..") || clean.contains('\\') {
            return Err(ServerError::Validation("invalid blob key traversal".into()));
        }
        Ok(self.root_dir.join(clean))
    }
}

impl BlobStorePort for LocalFsBlobStore {
    fn put_blob<'a>(
        &'a self,
        key: &'a str,
        data: Vec<u8>,
        _content_type: &'a str,
    ) -> BoxFuture<'a, Result<String, ServerError>> {
        Box::pin(async move {
            let path = self.resolve_path(key)?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).await.map_err(|e| {
                    ServerError::Internal(format!("failed to create blob dir: {e}"))
                })?;
            }
            fs::write(&path, &data)
                .await
                .map_err(|e| ServerError::Internal(format!("failed to write blob: {e}")))?;
            Ok(key.to_owned())
        })
    }

    fn get_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Vec<u8>, ServerError>> {
        Box::pin(async move {
            let path = self.resolve_path(key)?;
            fs::read(&path)
                .await
                .map_err(|e| ServerError::NotFound(format!("blob `{key}` not found: {e}")))
        })
    }

    fn delete_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let path = self.resolve_path(key)?;
            if path.exists() {
                fs::remove_file(&path)
                    .await
                    .map_err(|e| ServerError::Internal(format!("failed to remove blob: {e}")))?;
            }
            Ok(())
        })
    }

    fn presign_url<'a>(
        &'a self,
        key: &'a str,
        _expires_secs: u64,
    ) -> BoxFuture<'a, Result<String, ServerError>> {
        Box::pin(async move {
            let path = self.resolve_path(key)?;
            Ok(format!("file://{}", path.display()))
        })
    }
}

/// Upstash Blob REST adapter implementing [`BlobStorePort`].
#[derive(Clone)]
pub struct UpstashBlobStore {
    endpoint: String,
    token: String,
    client: reqwest::Client,
}

impl UpstashBlobStore {
    #[must_use]
    pub fn new(token: String) -> Self {
        Self {
            endpoint: "https://blob.upstash.com".to_owned(),
            token,
            client: reqwest::Client::new(),
        }
    }

    #[must_use]
    pub fn with_endpoint(endpoint: String, token: String) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            token,
            client: reqwest::Client::new(),
        }
    }
}

impl BlobStorePort for UpstashBlobStore {
    fn put_blob<'a>(
        &'a self,
        key: &'a str,
        data: Vec<u8>,
        content_type: &'a str,
    ) -> BoxFuture<'a, Result<String, ServerError>> {
        Box::pin(async move {
            let clean_key = key.trim_start_matches('/');
            let url = format!("{}/{}", self.endpoint, clean_key);
            let res = self
                .client
                .put(&url)
                .bearer_auth(&self.token)
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .body(data)
                .send()
                .await
                .map_err(|e| ServerError::Internal(format!("upstash blob put network: {e}")))?;
            if !res.status().is_success() {
                let status = res.status();
                let txt = res.text().await.unwrap_or_default();
                return Err(ServerError::Internal(format!(
                    "upstash blob put failed {status}: {txt}"
                )));
            }
            Ok(url)
        })
    }

    fn get_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Vec<u8>, ServerError>> {
        Box::pin(async move {
            let clean_key = key.trim_start_matches('/');
            let url = format!("{}/{}", self.endpoint, clean_key);
            let res = self
                .client
                .get(&url)
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(|e| ServerError::Internal(format!("upstash blob get network: {e}")))?;
            if !res.status().is_success() {
                let status = res.status();
                let txt = res.text().await.unwrap_or_default();
                return Err(ServerError::NotFound(format!(
                    "upstash blob get failed {status}: {txt}"
                )));
            }
            let bytes = res
                .bytes()
                .await
                .map_err(|e| ServerError::Internal(format!("upstash blob read bytes: {e}")))?;
            Ok(bytes.to_vec())
        })
    }

    fn delete_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let clean_key = key.trim_start_matches('/');
            let url = format!("{}/{}", self.endpoint, clean_key);
            let res = self
                .client
                .delete(&url)
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(|e| ServerError::Internal(format!("upstash blob delete network: {e}")))?;
            if !res.status().is_success() {
                let status = res.status();
                let txt = res.text().await.unwrap_or_default();
                return Err(ServerError::Internal(format!(
                    "upstash blob delete failed {status}: {txt}"
                )));
            }
            Ok(())
        })
    }

    fn presign_url<'a>(
        &'a self,
        key: &'a str,
        expires_secs: u64,
    ) -> BoxFuture<'a, Result<String, ServerError>> {
        Box::pin(async move {
            let clean_key = key.trim_start_matches('/');
            Ok(format!(
                "{}/{}?expires={expires_secs}",
                self.endpoint, clean_key
            ))
        })
    }
}

/// S3 / RustFS compatible HTTP object store implementing [`BlobStorePort`].
#[derive(Clone)]
pub struct S3BlobStore {
    endpoint: String,
    bucket: String,
    access_key: String,
    secret_key: String,
    client: reqwest::Client,
}

impl S3BlobStore {
    #[must_use]
    pub fn new(endpoint: String, bucket: String, access_key: String, secret_key: String) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            bucket: bucket.trim_matches('/').to_owned(),
            access_key,
            secret_key,
            client: reqwest::Client::new(),
        }
    }

    fn object_url(&self, key: &str) -> Result<String, ServerError> {
        let clean = key.trim_start_matches('/');
        if clean.contains("..") || clean.contains('\\') {
            return Err(ServerError::Validation(
                "invalid s3 blob key traversal".into(),
            ));
        }
        Ok(format!("{}/{}/{}", self.endpoint, self.bucket, clean))
    }
}

impl BlobStorePort for S3BlobStore {
    fn put_blob<'a>(
        &'a self,
        key: &'a str,
        data: Vec<u8>,
        content_type: &'a str,
    ) -> BoxFuture<'a, Result<String, ServerError>> {
        Box::pin(async move {
            let url = self.object_url(key)?;
            let res = self
                .client
                .put(&url)
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .header("x-amz-access-key", &self.access_key)
                .body(data)
                .send()
                .await
                .map_err(|e| ServerError::Internal(format!("s3 blob put network: {e}")))?;
            if !res.status().is_success() {
                let status = res.status();
                let txt = res.text().await.unwrap_or_default();
                return Err(ServerError::Internal(format!(
                    "s3 blob put failed {status}: {txt}"
                )));
            }
            Ok(url)
        })
    }

    fn get_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Vec<u8>, ServerError>> {
        Box::pin(async move {
            let url = self.object_url(key)?;
            let res = self
                .client
                .get(&url)
                .header("x-amz-access-key", &self.access_key)
                .send()
                .await
                .map_err(|e| ServerError::Internal(format!("s3 blob get network: {e}")))?;
            if !res.status().is_success() {
                let status = res.status();
                let txt = res.text().await.unwrap_or_default();
                return Err(ServerError::NotFound(format!(
                    "s3 blob get failed {status}: {txt}"
                )));
            }
            let bytes = res
                .bytes()
                .await
                .map_err(|e| ServerError::Internal(format!("s3 blob read bytes: {e}")))?;
            Ok(bytes.to_vec())
        })
    }

    fn delete_blob<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let url = self.object_url(key)?;
            let res = self
                .client
                .delete(&url)
                .header("x-amz-access-key", &self.access_key)
                .send()
                .await
                .map_err(|e| ServerError::Internal(format!("s3 blob delete network: {e}")))?;
            if !res.status().is_success() {
                let status = res.status();
                let txt = res.text().await.unwrap_or_default();
                return Err(ServerError::Internal(format!(
                    "s3 blob delete failed {status}: {txt}"
                )));
            }
            Ok(())
        })
    }

    fn presign_url<'a>(
        &'a self,
        key: &'a str,
        expires_secs: u64,
    ) -> BoxFuture<'a, Result<String, ServerError>> {
        Box::pin(async move {
            let url = self.object_url(key)?;
            let exp = crate::infra::state::now_ms() / 1000 + expires_secs;
            let sig = crate::modules::auth::services::compute_hmac_hex(
                self.secret_key.as_bytes(),
                format!("{url}:{exp}").as_bytes(),
            );
            Ok(format!(
                "{url}?X-Amz-Credential={}&X-Amz-Expires={expires_secs}&X-Amz-Signature={sig}",
                self.access_key
            ))
        })
    }
}
