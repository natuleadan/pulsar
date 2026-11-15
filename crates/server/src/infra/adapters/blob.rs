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
    region: String,
    access_key: String,
    secret_key: String,
    client: reqwest::Client,
}

impl S3BlobStore {
    #[must_use]
    pub fn new(endpoint: String, bucket: String, access_key: String, secret_key: String) -> Self {
        Self::with_region(
            endpoint,
            bucket,
            "us-east-1".to_owned(),
            access_key,
            secret_key,
        )
    }

    #[must_use]
    pub fn with_region(
        endpoint: String,
        bucket: String,
        region: String,
        access_key: String,
        secret_key: String,
    ) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            bucket: bucket.trim_matches('/').to_owned(),
            region,
            access_key,
            secret_key,
            client: reqwest::Client::new(),
        }
    }

    fn object_path_and_url(&self, key: &str) -> Result<(String, String), ServerError> {
        let clean = key.trim_start_matches('/');
        if clean.contains("..") || clean.contains('\\') {
            return Err(ServerError::Validation(
                "invalid s3 blob key traversal".into(),
            ));
        }
        let (_, prefix) = super::s3_sigv4::parse_endpoint(&self.endpoint);
        let uri_path = format!("{prefix}/{}/{}", self.bucket, clean);
        let url = format!("{}/{}/{}", self.endpoint, self.bucket, clean);
        Ok((uri_path, url))
    }

    /// Ensures the configured bucket exists in RustFS / S3 (`HEAD`, then `PUT` on 404).
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Internal`] if the S3 endpoint is unreachable or rejects bucket creation.
    pub async fn ensure_bucket(&self) -> Result<(), ServerError> {
        let (_, prefix) = super::s3_sigv4::parse_endpoint(&self.endpoint);
        let uri_path = format!("{prefix}/{}", self.bucket);
        let url = format!("{}/{}", self.endpoint, self.bucket);
        let head_sig = super::s3_sigv4::sign_request(
            "HEAD",
            &self.endpoint,
            &uri_path,
            &self.region,
            &self.access_key,
            &self.secret_key,
            b"",
        );
        let head = self
            .client
            .head(&url)
            .header("x-amz-date", head_sig.amz_date)
            .header("x-amz-content-sha256", head_sig.payload_sha256)
            .header(reqwest::header::AUTHORIZATION, head_sig.authorization)
            .send()
            .await
            .map_err(|e| ServerError::Internal(format!("s3 head bucket network: {e}")))?;
        if head.status().is_success() {
            return Ok(());
        }
        let put_sig = super::s3_sigv4::sign_request(
            "PUT",
            &self.endpoint,
            &uri_path,
            &self.region,
            &self.access_key,
            &self.secret_key,
            b"",
        );
        let put = self
            .client
            .put(&url)
            .header("x-amz-date", put_sig.amz_date)
            .header("x-amz-content-sha256", put_sig.payload_sha256)
            .header(reqwest::header::AUTHORIZATION, put_sig.authorization)
            .send()
            .await
            .map_err(|e| ServerError::Internal(format!("s3 create bucket network: {e}")))?;
        if !put.status().is_success() && put.status().as_u16() != 409 {
            let status = put.status();
            let txt = put.text().await.unwrap_or_default();
            return Err(ServerError::Internal(format!(
                "s3 create bucket failed {status}: {txt}"
            )));
        }
        Ok(())
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
            let (uri_path, url) = self.object_path_and_url(key)?;
            let sig = super::s3_sigv4::sign_request(
                "PUT",
                &self.endpoint,
                &uri_path,
                &self.region,
                &self.access_key,
                &self.secret_key,
                &data,
            );
            let res = self
                .client
                .put(&url)
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .header("x-amz-date", sig.amz_date)
                .header("x-amz-content-sha256", sig.payload_sha256)
                .header(reqwest::header::AUTHORIZATION, sig.authorization)
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
            let (uri_path, url) = self.object_path_and_url(key)?;
            let sig = super::s3_sigv4::sign_request(
                "GET",
                &self.endpoint,
                &uri_path,
                &self.region,
                &self.access_key,
                &self.secret_key,
                b"",
            );
            let res = self
                .client
                .get(&url)
                .header("x-amz-date", sig.amz_date)
                .header("x-amz-content-sha256", sig.payload_sha256)
                .header(reqwest::header::AUTHORIZATION, sig.authorization)
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
            let (uri_path, url) = self.object_path_and_url(key)?;
            let sig = super::s3_sigv4::sign_request(
                "DELETE",
                &self.endpoint,
                &uri_path,
                &self.region,
                &self.access_key,
                &self.secret_key,
                b"",
            );
            let res = self
                .client
                .delete(&url)
                .header("x-amz-date", sig.amz_date)
                .header("x-amz-content-sha256", sig.payload_sha256)
                .header(reqwest::header::AUTHORIZATION, sig.authorization)
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
            let _ = self.object_path_and_url(key)?;
            Ok(super::s3_sigv4::presign_get_url(
                &self.endpoint,
                &self.bucket,
                key,
                &self.region,
                &self.access_key,
                &self.secret_key,
                expires_secs,
            ))
        })
    }
}
