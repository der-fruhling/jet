use std::{path::{PathBuf, Path}, fs, fmt::LowerHex};

use colored::Colorize;
use futures::Future;
use lazy_static::lazy_static;
use meowhash::{MeowHasher, MeowHash};
use tempfile::{NamedTempFile, tempdir, TempDir};

lazy_static! {
    static ref TMP_DIR: TempDir = tempdir().expect("Failed to create temp directory for cache emulation");
}

const CONTENTS_DIR: &str = "contents";
const URL_DIR: &str = "by_url_hash";

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir().map_or_else(
        || TMP_DIR.path().to_path_buf(),
        |v| v.join("jet-cache")
    )
}

pub fn needs_cache_emulation() -> bool {
    dirs::cache_dir().is_none()
}

fn cached_url_as_name(hash: &MeowHash) -> String {
    format!("u.{:016x}.dat", hash.as_u128())
}

fn cached_contents_as_name(hash: &MeowHash) -> String {
    format!("f.{:016x}.dat", hash.as_u128())
}

pub fn cached_url_exists(url: &str) -> bool {
    fs::symlink_metadata(cache_dir()
        .join(URL_DIR)
        .join(cached_url_as_name(&MeowHasher::hash(url.as_bytes()))))
        .is_ok_and(|f| f.is_symlink())
}

fn url_real_path(url: &str) -> Result<PathBuf, std::io::Error> {
    cache_dir()
        .join(URL_DIR)
        .join(cached_url_as_name(&MeowHasher::hash(url.as_bytes())))
        .canonicalize()
}

pub enum CacheState {
    Hit { hash: u128 },
    Miss { bytes_downloaded: usize, hash: u128 }
}

pub async fn download<
    Fu: Future<Output = Result<Vec<u8>, Box<dyn std::error::Error>>>,
    F: FnOnce() -> Fu
>(url: &str, download: F) -> Result<(CacheState, Vec<u8>), Box<dyn std::error::Error>> {
    let url_path = cache_dir()
        .join(URL_DIR)
        .join(cached_url_as_name(&MeowHasher::hash(url.as_bytes())));
    
    if std::fs::symlink_metadata(&url_path).is_ok_and(|f| f.is_symlink()) {
        match url_path.canonicalize() {
            Ok(canon) => match tokio::fs::read(&canon).await {
                Ok(bytes) => {
                    let hash = MeowHasher::hash(&bytes[..]);
                    let contents_path = cache_dir()
                    .join(CONTENTS_DIR)
                    .join(cached_contents_as_name(&hash));

                    match contents_path.canonicalize() {
                        Ok(contents_path) => if canon == contents_path {
                            return Ok((CacheState::Hit { hash: hash.as_u128() }, bytes))
                        } else {
                            eprintln!("{}: file path {:?} does not match expected path {:?}", "warning".yellow(), &canon, &contents_path);
                            fs::remove_file(&url_path)?;
                        },
                        Err(err) => {
                            eprintln!("{}: error canonicalizing expected path {:?}: {}", "warning".yellow(), &contents_path, err);
                            fs::remove_file(&url_path)?;
                        },
                    };
                },
                Err(_) => {
                    eprintln!("{}: failed to read canon file {:?}", "warning".yellow(), &canon);
                    fs::remove_file(&url_path)?;
                    if fs::metadata(&canon).is_ok() {
                        fs::remove_file(&canon)?;
                    }
                },
            },
            Err(err) => {
                eprintln!("{}: failed to canonicalize existing URL symlink {:?}: {}", "warning".yellow(), &url_path, err);
            },
        };
    }
    
    let bytes = download().await?;
    let byte_len = bytes.len();
    let hash = MeowHasher::hash(&bytes[..]);

    let contents_path = cache_dir()
            .join(CONTENTS_DIR)
            .join(cached_contents_as_name(&hash));
    
    for path in [&contents_path, &url_path] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
    }
    
    let (cache_insert_result, cache_symlink_result) = tokio::join!(
        tokio::fs::write(&contents_path, &bytes),
        async {
            if fs::symlink_metadata(&url_path).is_ok() {
                return Ok(());
            }

            if let Ok(existing) = fs::metadata(&url_path) {
                if existing.is_file() {
                    tokio::fs::remove_file(&url_path).await?;
                } else if existing.is_dir() {
                    tokio::fs::remove_dir_all(&url_path).await?;
                }
            }
            
            tokio::fs::symlink_file(&contents_path, &url_path).await
        }
    );
    
    if let Err(err) = cache_insert_result {
        eprintln!("{}: failed to save cache data to {:?}: {:?}; future cachable requests will miss URL {}", "warning".yellow(), &contents_path, err, url);
    } else if let Err(err) = cache_symlink_result {
        eprintln!("{}: failed to create cache symlink to {:?} in {:?}: {:?}; future cachable requests will miss URL {}", "warning".yellow(), &contents_path, &url_path, err, url);
    }
    
    Ok((CacheState::Miss { bytes_downloaded: byte_len, hash: hash.as_u128() }, bytes))
}

pub async fn download_and_save<
    P: AsRef<Path>,
    Fu: Future<Output = Result<Vec<u8>, Box<dyn std::error::Error>>>,
    F: FnOnce() -> Fu
>(file_path: P, url: &str, download: F) -> Result<CacheState, Box<dyn std::error::Error>> {
    let (cache_state, bytes) = self::download(url, download).await?;
    tokio::fs::write(file_path, bytes).await?;
    Ok(cache_state)
}
