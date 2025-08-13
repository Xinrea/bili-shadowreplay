use std::path::Path;

use tokio::fs::File;

use crate::errors::ApiCollectionError;

/// Download image from url to temporary file
pub async fn download_image(
    client: &reqwest::Client,
    url: &str,
) -> Result<File, ApiCollectionError> {
    let response = client.get(url).send().await?;
    let bytes = response.bytes().await?;
    let mut temp_file = tempfile::tempfile()?;
    std::io::Write::write_all(&mut temp_file, &bytes)?;
    Ok(temp_file.into())
}

/// Download file from url to given path
pub async fn download_file(
    client: &reqwest::Client,
    url: &str,
    file_path: &Path,
) -> Result<u64, ApiCollectionError> {
    let res = client.get(url).send().await?;
    let mut file = tokio::fs::File::create(file_path).await?;
    let bytes = res.bytes().await?;
    let size = bytes.len() as u64;
    let mut content = std::io::Cursor::new(bytes);
    tokio::io::copy(&mut content, &mut file).await?;
    Ok(size)
}
