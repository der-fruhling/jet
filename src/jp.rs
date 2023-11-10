use std::{io::{Write, Read, Seek, SeekFrom}, path::{PathBuf, Path}, fs, collections::HashMap, str::FromStr, sync::Arc, fmt::Display};

use async_recursion::async_recursion;
use colored::Colorize;
use futures::future::join_all;
use once_cell::sync::Lazy;
use pathdiff::diff_paths;
use reqwest::{header::{HeaderValue, USER_AGENT, HeaderMap}, StatusCode};
use sha2::{Sha512, Digest};
use tar::Header;
use serde::{Serialize, Deserialize};
use tempfile::NamedTempFile;

use crate::{modrinth::{VersionFile, self}, cached::{self, CacheState}};

pub const EXTENSION: &'static str = "jpk";

static RUN_SCRIPT_MEM_PRESETS: Lazy<HashMap<String, String>> = Lazy::new(|| [
    ("none", ""),
    ("zgc", "-XX:+UseZGC -XX:AllocatePrefetchStyle=1 -XX:-ZProactive"),
    ("brucethemoose-server", "-XX:+UseG1GC -XX:MaxGCPauseMillis=130 -XX:+UnlockExperimentalVMOptions -XX:+DisableExplicitGC -XX:+AlwaysPreTouch -XX:G1NewSizePercent=28 -XX:G1HeapRegionSize=16M -XX:G1ReservePercent=20 -XX:G1MixedGCCountTarget=3 -XX:InitiatingHeapOccupancyPercent=10 -XX:G1MixedGCLiveThresholdPercent=90 -XX:G1RSetUpdatingPauseTimePercent=0 -XX:SurvivorRatio=32 -XX:MaxTenuringThreshold=1 -XX:G1SATBBufferEnqueueingThresholdPercent=30 -XX:G1ConcMarkStepDurationMillis=5 -XX:G1ConcRSHotCardLimit=16 -XX:G1ConcRefinementServiceIntervalMillis=150"),
    ("aikar", "-XX:+UseG1GC -XX:+ParallelRefProcEnabled -XX:MaxGCPauseMillis=200 -XX:+UnlockExperimentalVMOptions -XX:+DisableExplicitGC -XX:+AlwaysPreTouch -XX:G1NewSizePercent=30 -XX:G1MaxNewSizePercent=40 -XX:G1HeapRegionSize=8M -XX:G1ReservePercent=20 -XX:G1HeapWastePercent=5 -XX:G1MixedGCCountTarget=4 -XX:InitiatingHeapOccupancyPercent=15 -XX:G1MixedGCLiveThresholdPercent=90 -XX:G1RSetUpdatingPauseTimePercent=5 -XX:SurvivorRatio=32 -XX:+PerfDisableSharedMem -XX:MaxTenuringThreshold=1 -Dusing.aikars.flags=https://mcflags.emc.gs -Daikars.new.flags=true")
].map(|(k, v)| (k.into(), v.into())).into());

const RUN_TEMPLATE_SH: &str = include_str!("run.template.sh");
const RUN_TEMPLATE_BAT: &str = include_str!("run.template.bat");

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub java_mem: String,
    pub java_gc_opts: String,
    pub java_extra_opts: Vec<String>,
    pub server_jar_rel: String
}

impl Options {
    pub fn new() -> Self {
        Self {
            java_mem: "4G".into(),
            java_gc_opts: RUN_SCRIPT_MEM_PRESETS["brucethemoose-server"].clone(),
            java_extra_opts: vec![],
            server_jar_rel: "server.jar".into()
        }
    }
}

fn parse_template(template: &str, options: &Options) -> String {
    String::from(template)
        .replace("$$JAVA_MEM$$", &options.java_mem[..])
        .replace("$$JAVA_GC_OPTS$$", &options.java_gc_opts[..])
        .replace("$$JAVA_EXTRA_OPTS$$", &options.java_extra_opts.join(" ")[..])
        .replace("$$SERVER_JAR$$", &options.server_jar_rel[..])
}

// Required by Modrinth
const USER_AGENT_VALUE: &'static str = "der_fruhling/jet/0.1.0 (der_fruhling@outlook.com)";

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Default, Clone)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum ScriptType {
    Bash,
    Batch,
    #[default] Both
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum Entry {
    Directory { name: String, contents: Vec<Entry> },
    File { name: String, hash: u128, size: usize },
    Modrinth { project: String, version: String, files: Vec<VersionFile> },
    FabricServerJar { minecraft_version: String, loader_version: String, installer_version: String },
    RunScript {
        name: String,
        script_type: ScriptType,
        options: Options
    },
    Persist { name: String }
}

pub enum Action {
    CreateDir,
    Extract { hash: u128, size: usize },
    Download { display_name: String, url: String, sha512: Option<[u8; 64]> },
    Symlink { source: PathBuf },
    RunScriptTemplate { source: &'static str, options: Options },
    Persist
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub project_info: ProjectInfo,
    pub contents: Vec<Entry>
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub enum SourceRunOption {
    Memory {
        #[serde(rename = "@max")]
        memory: String
    },
    
    UseGc {
        #[serde(rename = "@preset")]
        preset: String
    },
    
    JavaArg(#[serde(rename = "$text")] String)
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub enum SourceEntry {
    Directory {
        #[serde(rename = "@name")]
        name: String,
        #[serde(rename = "$value")]
        contents: Vec<SourceEntry>
    },

    File {
        #[serde(rename = "@name")]
        name: String,
        #[serde(rename = "@from")]
        source_path: Option<PathBuf>
    },
    
    Modrinth {
        #[serde(rename = "@project")]
        project: String,
        #[serde(rename = "@version")]
        version: String
    },
    
    FabricServer {
        #[serde(rename = "@minecraft")]
        minecraft_version: String,
        #[serde(rename = "@loader")]
        loader_version: String,
        #[serde(rename = "@installer")]
        installer_version: String
    },
    
    RunScript {
        #[serde(rename = "@name")]
        name: String,
        #[serde(rename = "@type", default)]
        script_type: ScriptType,
        #[serde(rename = "$value")]
        options: Vec<SourceRunOption>
    },
    
    Persist {
        #[serde(rename = "@name")]
        name: String
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct ProjectInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    #[serde(rename = "author")]
    pub authors: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceManifest {
    pub project: ProjectInfo,
    
    #[serde(rename = "$value")]
    pub contents: Vec<SourceEntry>
}

impl Entry {
    #[async_recursion]
    async fn parse(value: &SourceEntry) -> Self {
        match value {
            SourceEntry::Directory { name, contents } => {
                Entry::Directory {
                    name: name.clone(),
                    contents: join_all(contents.iter().map(Entry::parse)).await
                }
            },
            SourceEntry::File { name, source_path } => {
                let data = fs::read(source_path.as_ref().unwrap())
                    .expect(format!("Failed to read file: {:?}", source_path.as_ref().unwrap().to_str()).as_str());
                
                Entry::File {
                    name: name.clone(),
                    hash: meowhash::MeowHasher::hash(&data).as_u128(),
                    size: data.len()
                }
            },
            SourceEntry::Modrinth { project, version } => {
                // need to resolve project id and version id into slugs
                let client = reqwest::Client::builder()
                    .default_headers(HeaderMap::from_iter([
                        (USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE))
                    ]))
                    .build().expect("Failed to build HTTP client");
                
                let version_resp = modrinth::project_version_get(&client, project, version).await;
                println!("{:>12} {} {} [version info]", "GET".magenta(), project, version);
                
                Entry::Modrinth {
                    project: project.clone(),
                    version: version.clone(),
                    files: version_resp.files
                }
            },
            SourceEntry::FabricServer { minecraft_version, loader_version, installer_version } => {
                Entry::FabricServerJar {
                    minecraft_version: minecraft_version.clone(),
                    loader_version: loader_version.clone(),
                    installer_version: installer_version.clone(),
                }
            },
            SourceEntry::RunScript { name, script_type, options } => {
                let mut opts = Options::new();
                
                for option in options {
                    match option {
                        SourceRunOption::Memory { memory } => opts.java_mem = memory.clone(),
                        SourceRunOption::UseGc { preset } => opts.java_gc_opts = if RUN_SCRIPT_MEM_PRESETS.contains_key(preset) {
                            RUN_SCRIPT_MEM_PRESETS[preset].clone()
                        } else {
                            eprintln!("{}: memory preset does not exist: {}", "warning".yellow(), preset);
                            RUN_SCRIPT_MEM_PRESETS["brucethemoose-server"].clone()
                        },
                        SourceRunOption::JavaArg(arg) => opts.java_extra_opts.push(arg.clone()),
                    }
                }
                
                Entry::RunScript {
                    name: name.clone(),
                    script_type: script_type.clone(),
                    options: opts
                }
            },
            SourceEntry::Persist { name } => {
                Entry::Persist { name: name.clone() }
            }
        }
    }
}

impl Manifest {
    pub async fn parse(value: &SourceManifest) -> Self {
        Self {
            project_info: value.project.clone(),
            contents: join_all(value.contents.iter().map(Entry::parse)).await
        }
    }
}

impl SourceEntry {
    fn resolve<P : AsRef<Path>>(&mut self, base_dir: P) {
        match self {
            SourceEntry::Directory { name, contents } => {
                let path = base_dir.as_ref().join(name);
                
                for child in contents {
                    child.resolve(&path);
                }
            },
            SourceEntry::File { name, source_path } => {
                if source_path.is_none() {
                    let path = base_dir.as_ref().join(name);
                    *source_path = Some(path);
                } else {
                    let source_path_data = source_path.take().unwrap();
                    let path = base_dir.as_ref().join(source_path_data);
                    *source_path = Some(path);
                }
            },
            SourceEntry::Modrinth { .. } => {}, // nothing to resolve
            SourceEntry::FabricServer { .. } => {}, // nothing to resolve
            SourceEntry::RunScript { .. } => {}, // nothing to resolve
            SourceEntry::Persist { .. } => {} // nothing to resolve
        }
    }
}

impl SourceManifest {
    pub fn resolve<P : AsRef<Path>>(&mut self, base_dir: P) {
        for content in &mut self.contents {
            content.resolve(&base_dir);
        }
    }
}

impl Manifest {
    fn as_actions<P : AsRef<Path>>(&self, base_dir: P) -> Vec<(PathBuf, Action)> {
        let mut actions = vec![];

        fn recurse_gen_actions(actions: &mut Vec<(PathBuf, Action)>, entry: &Entry, path: PathBuf) {
            match entry {
                Entry::Directory { contents, ..} => for child in contents {
                    actions.push((path.clone(), Action::CreateDir));
                    
                    recurse_gen_actions(actions, child, match child {
                        Entry::Directory { name, .. } => path.join(name),
                        Entry::File { name, .. } => path.join(name),
                        Entry::Modrinth { .. } => path.to_path_buf(), // projects can have multiple files
                        Entry::FabricServerJar { .. } => path.to_path_buf(), // TODO resolve
                        Entry::RunScript { .. } => path.to_path_buf(), // name can be templated
                        Entry::Persist { name } => path.join(name)
                    })
                },
                
                Entry::File { hash, size, .. } => {
                    actions.push((path, Action::Extract { hash: *hash, size: *size}));
                },
                
                Entry::Modrinth { project, version, files } => {
                    for file in files {
                        actions.push((
                            path.join(format!("{}-{}.{}",
                                project, version,
                                Path::new(&file.filename).extension().unwrap().to_str().unwrap()
                            )),
                            Action::Download {
                                display_name: format!("modrinth [{}-{}::{}]", project, version, file.filename),
                                url: file.url.clone(),
                                sha512: Some(hex::decode(&file.hashes.sha512)
                                    .expect("SHA-512 hash was not a valid hex string")
                                    .try_into().expect("SHA-512 hash was an invalid length"))
                            }
                        ))
                    }
                },
                
                Entry::FabricServerJar {
                    minecraft_version,
                    loader_version,
                    installer_version
                } => {
                    let server = path.join(format!("fabric-server.{}.{}.{}.jar", minecraft_version, loader_version, installer_version));
                    actions.push((
                        server.clone(),
                        Action::Download {
                            display_name: format!("fabric server [{}-{}, installer {}]", minecraft_version, loader_version, installer_version),
                            url: format!("https://meta.fabricmc.net/v2/versions/loader/{}/{}/{}/server/jar", minecraft_version, loader_version, installer_version),
                            sha512: None // fabric server does not provide hashes afaik
                        }
                    ));
                    
                    actions.push((
                        path.join("server.jar"),
                        Action::Symlink { source: server.clone() }
                    ));
                }
                
                Entry::RunScript { name, script_type, options } => {
                    match script_type {
                        ScriptType::Bash => {
                            actions.push((path.join(name), Action::RunScriptTemplate { source: RUN_TEMPLATE_SH, options: options.clone() }))
                        },

                        ScriptType::Batch => {
                            actions.push((path.join(name), Action::RunScriptTemplate { source: RUN_TEMPLATE_BAT, options: options.clone() }))
                        },

                        ScriptType::Both => {
                            actions.push((path.join(name.replace("%", "sh")), Action::RunScriptTemplate { source: RUN_TEMPLATE_SH, options: options.clone() }));
                            actions.push((path.join(name.replace("%", "bat")), Action::RunScriptTemplate { source: RUN_TEMPLATE_BAT, options: options.clone() }));
                        },
                    }
                },
                
                Entry::Persist { .. } => {
                    actions.push((path, Action::Persist))
                }
            }
        }
        
        for child in &self.contents {
            recurse_gen_actions(&mut actions, child, match child {
                Entry::Directory { name, .. } => base_dir.as_ref().join(name),
                Entry::File { name, .. } => base_dir.as_ref().join(name),
                Entry::Modrinth { .. } => base_dir.as_ref().to_path_buf(), // projects can have multiple files
                Entry::FabricServerJar { .. } => base_dir.as_ref().to_path_buf(), // TODO resolve
                Entry::RunScript { .. } => base_dir.as_ref().to_path_buf(), // name can be templated
                Entry::Persist { name } => base_dir.as_ref().join(name)
            })
        }

        actions
    }
}

fn add_data<W : Write, R : Read>(builder: &mut tar::Builder<W>, path: &str, mut contents: R) {
    let mut vec = Vec::new();
    contents.read_to_end(&mut vec)
        .expect(format!("Failed to read for file {}", path).as_str());
    
    let mut header = Header::new_gnu();
    header.set_size(vec.len() as u64);
    header.set_cksum();
    
    builder.append_data(&mut header, path, &vec[..])
        .expect(format!("Failed to append file {}", path).as_str());
}

fn recurse_files<F : FnMut(&SourceEntry)>(entry: &SourceEntry, f: &mut F) {
    match entry {
        SourceEntry::Directory { contents, .. } => {
            for child in contents {
                recurse_files(child, f);
            }
        },
        SourceEntry::File { .. } => f(entry),
        _ => {}
    }
}

pub async fn pack<W : Write, P1 : AsRef<Path>, P2 : AsRef<Path>>(writer: W, manifest_path: Option<P1>, mut manifest: SourceManifest, source_dir: P2) {
    manifest.resolve(source_dir);
    
    let mut builder = tar::Builder::new(writer);
    builder.follow_symlinks(false);
    
    println!("{:>12} @manifest", "Generating".green());
    
    let mut data = Vec::new();
    ciborium::into_writer(&Manifest::parse(&manifest).await, &mut data)
        .expect("Failed to serialize manifest");

    println!("{:>12} @manifest", "Writing".yellow());
    add_data(&mut builder, "@manifest", &data[..]);
    
    if let Some(path) = manifest_path {
        println!("{:>12} @jetfuel.xml", "Writing".yellow());
        let data = fs::read(&path).expect(&format!("Failed to read from {}", path.as_ref().to_str().unwrap()));
        add_data(&mut builder, "@jetfuel.xml", &data[..]);
    }
    
    for child in &manifest.contents {
        recurse_files(child, &mut |file| {
            let SourceEntry::File { source_path, .. } = file else {
                return;
            };

            print!("{:>12} {}", "Embedding".yellow(), source_path.as_ref().unwrap()
                .to_str().unwrap_or_else(|| "<unknown>"));
            
            let data = fs::read(source_path.as_ref().unwrap())
                .expect(format!("Failed to read {:?}", source_path.as_ref().unwrap().to_str()).as_str());
            let hash = meowhash::MeowHasher::hash(&data[..]);
            let filename = format!("{:032x}", hash.as_u128());
            
            print!(" (hash: {})", filename.as_str());
            
            let mut header = Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_cksum();
            
            builder.append_data(&mut header, &filename, &data[..])
                .expect(format!("Failed to append data hash {}", &filename).as_str());
            println!()
        });
    }
    
    println!("{:>12} archive", "Finishing".green());
    builder.into_inner().expect("Failed to save archive");
}

pub fn unpack<R : Read, P : AsRef<Path>>(reader: R, target_dir: P) {
    let mut archive = tar::Archive::new(reader);
    archive.unpack(target_dir.as_ref()).expect("Failed to unpack archive");
    
    println!("{:>12} into {}", "Unpacked".blue(), target_dir.as_ref().to_str().unwrap());
    
    if let Ok(bytes) = fs::read(target_dir.as_ref().join("@manifest")) {
        let manifest: Manifest = match ciborium::from_reader(&bytes[..]) {
            Ok(manifest) => manifest,
            Err(err) => {
                eprintln!("{}: error reading @manifest: {}", "warning".yellow(), err);
                return;
            }
        };
        
        let writer = fs::File::create(target_dir.as_ref().join("@manifest.json"))
            .expect("Failed to create JSON file");
        
        if let Err(err) = serde_json::to_writer_pretty(writer, &manifest) {
            eprintln!("{}: error writing @manifest as JSON: {}", "warning".yellow(), err);
            return;
        }

        println!("{:>12} @manifest into human-readable JSON", "Converted".blue());
    }
}

pub fn unpack_selective<R : Read>(reader: R, name: &str) -> Option<Vec<u8>> {
    let mut archive = tar::Archive::new(reader);

    for entry in archive.entries().expect("Failed to read entries from tar archive") {
        let mut entry = entry.expect("Failed to read tar entry");
        if entry.path().unwrap().file_name().unwrap().to_str().unwrap() == name {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).expect("Failed to read tar entry data");
            return Some(buf);
        }
    }
    
    None
}

pub async fn expand<R : Read, P : AsRef<Path>>(reader: R, target_dir: P) {
    fs::create_dir_all(target_dir.as_ref()).expect("Failed to create directory");
    
    let mut archive = tar::Archive::new(reader);
    let mut entries = archive.entries()
        .expect("Failed to read tar file");
    
    println!("{:>12} manifest", "Reading".blue());
    
    let manifest_entry = entries.next();
    let Some(manifest_entry) = manifest_entry else {
        panic!("Jetpacked archive must include at least one file");
    };
    
    if let Err(e) = manifest_entry {
        panic!("Failed to read first file of Jetpacked archive: {}", e);
    }
    
    let manifest_entry = manifest_entry.unwrap();
    
    if manifest_entry.path().unwrap().to_str().expect("Strange path could not be converted to string") != "@manifest" {
        panic!("First file in Jetpacked archive must be @manifest");
    }
    
    let manifest: Manifest = ciborium::from_reader(manifest_entry)
        .expect("Failed to read @manifest");
    
    let mut persist_file_name = NamedTempFile::new().expect("Failed to create temporary file");
    println!("{:>12} {}", "Persist File".yellow(), persist_file_name.path().to_str().unwrap());
    
    let persist_file = persist_file_name.as_file_mut();
    let mut persist = tar::Builder::new(persist_file);
    
    let mut extract_map = HashMap::<PathBuf, PathBuf>::new();
    let mut join_handles = vec![];
    let client = Arc::new(reqwest::Client::builder()
        .default_headers(HeaderMap::from_iter([
            (USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE))
        ]))
        .build()
        .expect("Failed to build HTTP client"));
    
    let actions = manifest.as_actions(&target_dir);
    
    for (path, action) in &actions {
        if let Action::Persist = action {
            match fs::File::open(path) {
                Ok(mut file) => {
                    persist.append_file(diff_paths(path, &target_dir).unwrap_or_else(|| path.clone()), &mut file).expect(&format!("Failed to persist file {}", path.to_str().unwrap()));
                    println!("{:>12} {}", "Persist".yellow(), path.to_str().unwrap());
                }
                
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::NotFound {
                        println!("{:>12} {} (nothing to persist)", "Persist".yellow().strikethrough(), path.to_str().unwrap());
                    } else {
                        panic!("Failed to persist {}: {}", path.to_str().unwrap(), err);
                    }
                }
            }
        }
    }
    
    let persist_file = persist.into_inner().expect("Failed to finish persistance file");
    persist_file.seek(SeekFrom::Start(0)).expect("Failed to seek in persistance file");
    let mut persist = tar::Archive::new(persist_file);
    
    if let Err(err) = fs::remove_dir_all(&target_dir) {
        eprintln!("{}: failed to remove target directory; output may be dirty: {}", "warning".yellow(), err);
    }
    
    for (path, action) in actions {
        match action {
            Action::CreateDir => fs::create_dir_all(path).expect("Failed to create directory"),
            Action::Extract { hash, .. } => {
                extract_map.insert(PathBuf::from_str(&format!("{:032x}", hash)).unwrap(), path);
            },
            Action::Download { display_name, url, sha512 } => {
                let client = client.clone();
                join_handles.push(tokio::spawn(async move {
                    #[derive(Debug)]
                    struct PhonyError;

                    impl Display for PhonyError {
                        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                            write!(f, "[phony error]")
                        }
                    }

                    impl std::error::Error for PhonyError {}

                    println!("{:>12} [{}] {} -> {} (url: {})", "GET".magenta(), "start".magenta(), display_name, path.to_str().unwrap(), &url);
                    
                    let bytes = cached::download(&url[..], || async {
                        let response = client.get(&url).send().await
                            .expect(&format!("Failed to GET {}", &url));

                        match response.status() {
                            StatusCode::OK => {
                                let bytes = response.bytes().await;
                                if let Err(err) = bytes {
                                    println!("{:>12} [{}] {} -> {} (url: {})", "GET".magenta(), "FAILED".red(), display_name, path.to_str().unwrap(), &url);
                                    eprintln!("GET {} failed with error: {}", &url, err);
                                    return Err::<Vec<u8>, Box<dyn std::error::Error>>(Box::new(PhonyError));
                                }

                                Ok(bytes.unwrap().into())
                            },
                            StatusCode::NOT_FOUND => {
                                println!("{:>12} [{}] {} -> {} (url: {})", "GET".magenta(), "FAILED".red(), display_name, path.to_str().unwrap(), &url);
                                eprintln!("GET {} was not found", &url);
                                return Err::<Vec<u8>, Box<dyn std::error::Error>>(Box::new(PhonyError));
                            },
                            status => {
                                println!("{:>12} [{}] {} -> {} (url: {})", "GET".magenta(), "FAILED".red(), display_name, path.to_str().unwrap(), &url);
                                eprintln!("GET {} returned random status code {}", &url, status);
                                return Err::<Vec<u8>, Box<dyn std::error::Error>>(Box::new(PhonyError));
                            }
                        }
                    }).await;

                    let Ok((cache_state, bytes)) = bytes else {
                        return false
                    };

                    if let CacheState::Miss { bytes_downloaded, hash } = cache_state {
                        println!("{:>12} (downloaded {} bytes as {:016x})", "Cache Miss".magenta(), bytes_downloaded, hash);
                    }
                    
                    let bytes: Vec<u8> = bytes.bytes()
                        .map(|r| r.unwrap_or_else(|e| panic!("Data failed to read: {}", e)))
                        .collect();
                    
                    if let Some(sha512) = sha512 {
                        let mut sha512_downloaded = Sha512::new();
                        sha512_downloaded.update(&bytes);
                        let result = sha512_downloaded.finalize();

                        if result[..] != sha512 {
                            println!("{:>12} [{}] {} -> {} (url: {})", "GET".magenta(), "FAILED".red(), display_name, path.to_str().unwrap(), &url);
                            eprintln!("File {} failed SHA-512 check (downloaded: {:?}, expected: {:?})", &url, &result[..], &sha512);
                            return false;
                        } 
                    }
                    
                    fs::write(&path, bytes)
                        .expect(&format!("Failed to write file {}", path.to_str().unwrap()));

                    println!("{:>12} [{}] {} -> {} (url: {})", "GET".magenta(), "success".green(), display_name, path.to_str().unwrap(), &url);
                    
                    true
                }))
            },
            Action::Symlink { source } => {
                match symlink::symlink_file(source, path) {
                    Ok(()) => {},
                    Err(err) => {
                        eprintln!("{}: failed to symlink server.jar: {}", "error".red(), err);
                    },
                };
            },
            Action::RunScriptTemplate { source, options } => {
                if let Err(err) = fs::write(&path, parse_template(source, &options)) {
                    eprintln!("{}: failed to write {:?}: {}", "error".red(), &path, err);
                }
            }
            Action::Persist => {}
        }
    }
    
    let mut extract_errors = false;
    
    for entry in entries {
        if let Err(err) = entry {
            eprintln!("Error reading entry: {}", err);
            extract_errors = true;
            continue;
        }
        
        let mut entry = entry.unwrap();
        let path_buf = entry.path().unwrap().to_path_buf();
        
        if let Some(target) = extract_map.get(&path_buf) {
            println!("{:>12} {} -> {}", "Extract".green(), path_buf.to_str().unwrap(), target.to_str().unwrap());
            
            let mut contents = Vec::new();
            entry.read_to_end(&mut contents)
                .expect(&format!("Failed to read from archive file {:?}", entry.path().unwrap()));
            fs::write(target, contents)
                .expect(&format!("Failed to write to file {:?}", target));
            
            extract_map.remove(&path_buf);
        }
    }
    
    let results = join_all(join_handles).await;
    let entries = persist.entries_with_seek();
    
    match entries {
        Ok(entries) => for entry in entries {
            match entry {
                Ok(mut entry) => {
                    println!("{:>12} {}", "Restore".blue(), entry.path().unwrap().to_str().unwrap());
                    let mut buf = Vec::new();
                    if let Err(err) = entry.read_to_end(&mut buf) {
                        error_unpersist(err, persist);
                        return;
                    }
                    
                    if let Some(parent) = entry.path().unwrap().parent() {
                        if let Err(err) = fs::create_dir_all(parent) {
                            eprintln!("{}: failed to create directory {}: {}", "warning".yellow(), parent.to_str().unwrap(), err);
                        }
                    }
                    
                    if let Err(err) = fs::write(entry.path().unwrap(), &buf) {
                        error_unpersist(err, persist);
                        return;
                    }
                }

                Err(err) => {
                    error_unpersist(err, persist);
                    return;
                }
            }
        }
        
        Err(err) => {
            error_unpersist(err, persist);
            return;
        }
    }
    
    for result in results {
        match result {
            Ok(true) => { /* OK */ },
            Ok(false) => eprintln!("Unpacked target is definitely incomplete due to above GET errors"),
            Err(err) => eprintln!("Failed to join a future: {}", err)
        }
    }
    
    if extract_errors {
        eprintln!("Extract errors are present (your jetpacked archive is probably corrupt)");
    }
}

fn error_unpersist(err: std::io::Error, persist: tar::Archive<&mut fs::File>) {
    eprintln!("{}: failed to restore persisted files: {}", "error".red(), err);
    let name = format!("persisted.{}.tar", hex::encode(rand::random::<[u8; 16]>()));
    let mut buf = Vec::new();
    let file = persist.into_inner();
    file.seek(SeekFrom::Start(0)).expect("Failed to restore persisted files: seek failed");
    file.read_to_end(&mut buf).expect("Failed to restore persisted files: read failed");
    fs::write(&name, &buf).expect(&format!("Failed to restore persisted files: write to {} failed", &name));
    eprintln!("{} saved to {}", "archive of all persisted files".bold(), name);
}
