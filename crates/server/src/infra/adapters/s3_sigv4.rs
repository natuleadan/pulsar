//! Minimal AWS Signature Version 4 (SigV4) signer and presigned URL builder
//! for S3 / RustFS compatible object storage.

use chrono::Utc;
use sha2::{Digest, Sha256};

/// Computes SHA-256 hex digest of `data`.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Computes RFC 2104 HMAC-SHA256 digest.
#[must_use]
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k_pad = [0_u8; BLOCK];
    if key.len() > BLOCK {
        let hashed = Sha256::digest(key);
        k_pad[..32].copy_from_slice(&hashed);
    } else {
        k_pad[..key.len()].copy_from_slice(key);
    }
    let mut i_pad = [0x36_u8; BLOCK];
    let mut o_pad = [0x5c_u8; BLOCK];
    for i in 0..BLOCK {
        i_pad[i] ^= k_pad[i];
        o_pad[i] ^= k_pad[i];
    }
    let mut inner = Sha256::new();
    inner.update(i_pad);
    inner.update(msg);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(o_pad);
    outer.update(inner_hash);
    outer.finalize().into()
}

fn signing_key(secret_key: &str, date_stamp: &str, region: &str) -> [u8; 32] {
    let k_date = hmac_sha256(
        format!("AWS4{secret_key}").as_bytes(),
        date_stamp.as_bytes(),
    );
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, b"s3");
    hmac_sha256(&k_service, b"aws4_request")
}

/// Extracts `(host_header, base_path_prefix)` from an S3 endpoint URL.
#[must_use]
pub fn parse_endpoint(endpoint: &str) -> (&str, &str) {
    let stripped = endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))
        .unwrap_or(endpoint);
    match stripped.find('/') {
        Some(idx) => (&stripped[..idx], stripped[idx..].trim_end_matches('/')),
        None => (stripped, ""),
    }
}

/// Percent-encodes a URI path segment or query value per AWS SigV4 rules.
#[must_use]
pub fn aws_uri_encode(input: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        let unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if unreserved || (!encode_slash && b == b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Signed request headers for S3 HTTP calls.
pub struct SigV4Headers {
    pub amz_date: String,
    pub payload_sha256: String,
    pub authorization: String,
}

/// Signs an S3 HTTP request using AWS SigV4 `Authorization` header.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn sign_request(
    method: &str,
    endpoint: &str,
    uri_path: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
    payload: &[u8],
) -> SigV4Headers {
    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();
    let (host, _) = parse_endpoint(endpoint);
    let payload_sha256 = sha256_hex(payload);
    let canonical_uri = aws_uri_encode(uri_path, false);
    let canonical_headers =
        format!("host:{host}\nx-amz-content-sha256:{payload_sha256}\nx-amz-date:{amz_date}\n");
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_sha256}"
    );

    let scope = format!("{date_stamp}/{region}/s3/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let key = signing_key(secret_key, &date_stamp, region);
    let signature = hex::encode(hmac_sha256(&key, string_to_sign.as_bytes()));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );
    SigV4Headers {
        amz_date,
        payload_sha256,
        authorization,
    }
}

/// Builds an AWS SigV4 presigned `GET` URL valid for `expires_secs`.
#[must_use]
pub fn presign_get_url(
    endpoint: &str,
    bucket: &str,
    key: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
    expires_secs: u64,
) -> String {
    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();
    let (host, prefix) = parse_endpoint(endpoint);
    let clean_key = key.trim_start_matches('/');
    let raw_path = format!("{prefix}/{bucket}/{clean_key}");
    let canonical_uri = aws_uri_encode(&raw_path, false);
    let scope = format!("{date_stamp}/{region}/s3/aws4_request");
    let credential = aws_uri_encode(&format!("{access_key}/{scope}"), true);
    let ttl = expires_secs.clamp(1, 604_800);
    let canonical_query = format!(
        "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential={credential}&X-Amz-Date={amz_date}&X-Amz-Expires={ttl}&X-Amz-SignedHeaders=host"
    );
    let canonical_headers = format!("host:{host}\n");
    let canonical_request = format!(
        "GET\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\nhost\nUNSIGNED-PAYLOAD"
    );
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let signing = signing_key(secret_key, &date_stamp, region);
    let signature = hex::encode(hmac_sha256(&signing, string_to_sign.as_bytes()));
    format!(
        "{}/{bucket}/{clean_key}?{canonical_query}&X-Amz-Signature={signature}",
        endpoint.trim_end_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigv4_headers_and_presign_url_include_standard_aws_parameters() {
        let signed = sign_request(
            "PUT",
            "http://127.0.0.1:9000",
            "/agents-artifacts/tasks/s1/out.txt",
            "us-east-1",
            "rustfsadmin",
            "rustfssecret",
            b"hello",
        );
        assert!(
            signed
                .authorization
                .starts_with("AWS4-HMAC-SHA256 Credential=rustfsadmin/")
        );
        assert!(
            signed
                .authorization
                .contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date")
        );
        assert_eq!(signed.payload_sha256.len(), 64);

        let url = presign_get_url(
            "http://127.0.0.1:9000",
            "agents-artifacts",
            "tasks/s1/out.txt",
            "us-east-1",
            "rustfsadmin",
            "rustfssecret",
            300,
        );
        assert!(url.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(url.contains("X-Amz-Expires=300"));
        assert!(url.contains("X-Amz-SignedHeaders=host"));
        assert!(url.contains("X-Amz-Signature="));
    }
}
