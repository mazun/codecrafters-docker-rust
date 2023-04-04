use std::{collections::HashMap, io::Write, path::Path};
use tempfile::TempDir;

use anyhow::anyhow;
use bytes::Buf;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct DockerAPI {
    base_url: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct AuthInfo {
    token: String,
    expires_in: i32,
    issued_at: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct OCIImageIndexV1Platform {
    architecture: String,
    os: String,
    variant: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct OCIImageIndexV1Manifest {
    digest: String,
    #[serde(rename = "mediaType")]
    media_type: String,
    platform: OCIImageIndexV1Platform,
    size: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct OCIImageIndexV1 {
    manifests: Vec<OCIImageIndexV1Manifest>,
    #[serde(rename = "mediaType")]
    media_type: String,
    #[serde(rename = "schemaVersion")]
    schema_version: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct DockerFSLayers {
    #[serde(rename = "blobSum")]
    blob_sum: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct DockerManifestV1 {
    name: String,
    tag: String,
    architecture: String,
    #[serde(rename = "fsLayers")]
    fs_layers: Vec<DockerFSLayers>,
}

impl DockerAPI {
    pub fn new() -> DockerAPI {
        DockerAPI {
            base_url: "https://registry.hub.docker.com".to_owned(),
        }
    }

    async fn authenticate(&self, url: &str) -> anyhow::Result<AuthInfo> {
        let response = reqwest::get(url).await?;
        if let Some(auth) = response.headers().get("www-authenticate") {
            let auth_info = String::from_utf8(auth.as_bytes().to_vec())?;
            if let Some((bearer_realm, service, scope)) =
                (|| -> Option<(String, String, String)> {
                    let auth_hash: HashMap<String, String> = auth_info
                        .split(",")
                        .map(|s| {
                            let pos = s.find("=").unwrap();
                            let key = &s[..pos];
                            let value = &s[(pos + 2)..(s.len() - 1)]; // Remove parenthes
                            (key.to_string(), value.to_string())
                        })
                        .collect();
                    Some((
                        auth_hash.get("Bearer realm")?.to_string(),
                        auth_hash.get("service")?.to_string(),
                        auth_hash.get("scope")?.to_string(),
                    ))
                })()
            {
                let auth_url = reqwest::Url::parse_with_params(
                    &format!("{}", bearer_realm),
                    [("service", service), ("scope", scope)],
                )?;
                let auth_info: AuthInfo = reqwest::get(auth_url).await?.json().await?;
                Ok(auth_info)
            } else {
                Err(anyhow!("Auth information not found"))
            }
        } else {
            Err(anyhow!("Auth information not found"))
        }
    }

    pub async fn pull(&self, name: &str, reference: &str, dest_dir: &Path) -> anyhow::Result<()> {
        let manifest_url = format!(
            "{}/v2/library/{}/manifests/{}",
            self.base_url, name, reference
        );
        let auth = self.authenticate(&manifest_url).await?;
        let client = reqwest::Client::new();
        let manifest = client
            .get(manifest_url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", auth.token),
            )
            // .header("Accept", "application/vnd.oci.image.index.v1+json")
            .send()
            .await?;
        // eprintln!("{:?}", manifest);
        let tmp_dir = TempDir::new()?;
        match manifest.headers().get("content-type").unwrap().as_bytes() {
            b"application/vnd.docker.distribution.manifest.v1+prettyjws" => {
                let manifest: DockerManifestV1 = manifest.json().await?;
                for layer in manifest.fs_layers {
                    let digest: Vec<&str> = layer.blob_sum.split(":").collect();
                    let client = reqwest::Client::new();
                    // eprintln!("pulling {}", layer.blob_sum);
                    let blob = client
                        .get(format!(
                            "{}/v2/library/{}/blobs/{}:{}",
                            self.base_url, name, digest[0], digest[1]
                        ))
                        .header(
                            reqwest::header::AUTHORIZATION,
                            format!("Bearer {}", auth.token),
                        )
                        .header("Accept", "application/vnd.oci.image.index.v1+json")
                        .send()
                        .await?;
                    let blob_file_path = tmp_dir.path().join(digest[1]);
                    let blob_file_path_str = blob_file_path.to_str().unwrap();
                    // eprintln!("Blob: {:?}", blob);
                    {
                        let blob = blob.bytes().await?;
                        let mut blob_file = std::fs::File::create(blob_file_path_str)?;
                        blob_file.write(blob.chunk())?;
                    }
                    let output = std::process::Command::new("tar")
                        .stdout(std::process::Stdio::inherit())
                        .stderr(std::process::Stdio::inherit())
                        .args(["xzf", blob_file_path_str, "-C", dest_dir.to_str().unwrap()])
                        .output()?;
                    if !output.status.success() {
                        return Err(anyhow::anyhow!("Failed to unarchive {}", digest[1]));
                    }
                }
            }
            f => {
                return Err(anyhow::anyhow!(
                    "Unknown manifest format {}",
                    String::from_utf8(f.to_vec()).unwrap()
                ));
            }
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pull() -> anyhow::Result<()> {
        let api = DockerAPI::new();
        let tmp_dir = TempDir::new()?;
        let res = api.pull("alpine", "latest", tmp_dir.path()).await;
        eprintln!("{:?}", res);
        assert!(res.is_ok());
        Ok(())
    }
}
