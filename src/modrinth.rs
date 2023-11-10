// simple and small modrinth api stuff

use colored::Colorize;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::cached::CacheState;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum SideSupport {
    Required,
    Optional,
    Unsupported
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalStatus {
    Approved,
    Archived,
    Rejected,
    Draft,
    Unlisted,
    Processing,
    Withheld,
    Scheduled,
    Private,
    Unknown
}

#[derive(Deserialize, Debug)]
pub struct DonationUrl {
    pub id: String,
    pub platform: String,
    pub url: String
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectType {
    Mod,
    #[serde(rename = "modpack")]
    ModPack,
    #[serde(rename = "resourcepack")]
    ResourcePack,
    Shader
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum MonetizationStatus {
    Monetized,
    Demonetized,
    ForceDemonitized
}

#[derive(Deserialize, Debug)]
pub struct ModeratorMessage {
    pub message: String,
    pub body: Option<String>
}

#[derive(Deserialize, Debug)]
pub struct License {
    pub id: String,
    pub name: String,
    pub url: String
}

#[derive(Deserialize, Debug)]
pub struct GalleryEntry {
    pub url: String,
    pub featured: bool,
    pub title: Option<String>,
    pub description: Option<String>,
    pub created: String,
    pub ordering: usize
}

#[derive(Deserialize, Debug)]
pub struct ProjectGetResponse {
    pub slug: String,
    pub title: String,
    pub description: String,
    pub categories: Vec<String>,
    pub client_side: SideSupport,
    pub server_side: SideSupport,
    pub body: String,
    pub status: ApprovalStatus,
    pub requested_status: Option<ApprovalStatus>,
    pub additional_categories: Option<Vec<String>>,
    pub issues_url: Option<String>,
    pub source_url: Option<String>,
    pub wiki_url: Option<String>,
    pub discord_url: Option<String>,
    pub donation_urls: Option<Vec<DonationUrl>>,
    pub project_type: ProjectType,
    pub downloads: isize,
    pub icon_url: Option<String>,
    pub color: Option<i32>,
    pub thread_id: Option<String>,
    pub monetization_status: MonetizationStatus,
    pub id: String,
    pub team: String,
    pub body_url: Option<String>,
    pub moderator_message: Option<ModeratorMessage>,
    pub published: String,
    pub updated: String,
    pub approved: Option<String>,
    pub queued: Option<String>,
    pub followers: usize,
    pub license: Option<License>,
    pub versions: Option<Vec<String>>,
    pub game_versions: Option<Vec<String>>,
    pub loaders: Option<Vec<String>>,
    pub gallery: Option<Vec<GalleryEntry>>
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyType {
    Required,
    Optional,
    Incompatible,
    Embedded
}

#[derive(Deserialize, Serialize, Debug)]
pub struct DependencyEntry {
    pub version_id: Option<String>,
    pub project_id: Option<String>,
    pub file_name: Option<String>,
    pub dependency_type: DependencyType
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum VersionType {
    Release,
    Beta,
    Alpha
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Eq)]
pub struct VersionFileHashes {
    pub sha512: String,
    pub sha1: String
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VersionFileType {
    RequiredResourcePack,
    OptionalResourcePack
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Eq)]
pub struct VersionFile {
    pub hashes: VersionFileHashes,
    pub url: String,
    pub filename: String,
    pub primary: bool,
    pub size: usize,
    pub file_type: Option<VersionFileType>
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum VersionStatus {
    Listed,
    Archived,
    Draft,
    Unlisted,
    Scheduled,
    Unknown
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ProjectVersionGetResponse {
    pub name: String,
    pub version_number: String,
    pub changelog: Option<String>,
    pub dependenices: Option<Vec<DependencyEntry>>,
    pub game_versions: Vec<String>,
    pub version_type: VersionType,
    pub loaders: Vec<String>,
    pub featured: bool,
    pub status: Option<VersionStatus>,
    pub requested_status: Option<VersionStatus>,
    pub id: String,
    pub project_id: String,
    pub author_id: String,
    pub date_published: String,
    pub downloads: usize,
    pub changelog_url: Option<String>,
    pub files: Vec<VersionFile>
}

pub async fn project_version_get(
    client: &reqwest::Client,
    project: &str,
    version: &str
) -> ProjectVersionGetResponse {
    let url = format!("https://api.modrinth.com/v2/project/{}/version/{}", project, version);
    let (cache_state, bytes) = crate::cached::download(&url.clone()[..], move || async move {
        let response = client.get(url)
                    .send().await
                    .expect(&format!("Failed to GET version info of {} {}", project, version));

        match response.status() {
            StatusCode::OK => {
                let bytes: Vec<u8> = response.bytes().await.expect("Could not read bytes from Modrinth version request").into();
                let response = serde_json::from_slice::<ProjectVersionGetResponse>(&bytes[..])
                    .expect("Failed to deserialize ProjectVersionGetResponse");

                let mut real_bytes = Vec::new();
                ciborium::into_writer(&response, &mut real_bytes)
                    .expect("Failed to serialize ProjectVersionGetResponse");
                Ok(real_bytes)
            },
            StatusCode::NOT_FOUND => {
                panic!("Unknown Modrinth version {} {}", project, version);
            },
            status => panic!("Random status code getting Modrinth version {} {}: {:?}", project, version, status)
        }
    }).await.expect("Failed to get Modrinth version info");

    if let CacheState::Miss { bytes_downloaded, hash } = cache_state {
        println!("{:>12} (downloaded {} bytes as {:016x})", "Cache Miss".magenta(), bytes_downloaded, hash);
    }

    ciborium::from_reader(&bytes[..])
        .expect("Failed to deserialize ProjectVersionGetResponse")
}
