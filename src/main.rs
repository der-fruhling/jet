mod jp;
mod jp_zlib;
mod modrinth;
mod cached;

use std::{path::{PathBuf, Path}, fs};
use std::io::{Read, stdin, stdout, Write};

use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use jp::SourceManifest;
use syntect::{parsing::SyntaxSet, highlighting::{ThemeSet, Style}, easy::HighlightLines, util::{LinesWithEndings, as_24_bit_terminal_escaped}};
use crate::cached::cache_dir;

#[derive(Parser)]
#[command(about, author, version)]
struct Cli {
    #[command(subcommand)]
    subcommand: SubCommand
}

#[derive(ValueEnum, Clone)]
enum Compression {
    None,
    Zlib
}

#[derive(Clone, Subcommand)]
enum SubCommand {
    Pack {
        #[arg(default_value = ".")]
        source: PathBuf,
        
        #[arg(short, long)]
        output: PathBuf,
        
        #[arg(short = 'F', long)]
        jetfuel_path: Option<PathBuf>,
        
        #[arg(short = 'c', long, default_value = "zlib")]
        compression: Compression
    },
    Unpack {
        #[arg(short, long)]
        source: PathBuf,

        #[arg(short, long)]
        output: PathBuf,

        #[arg(short = 'c', long, default_value = "zlib")]
        compression: Option<Compression>
    },
    Peek {
        file: PathBuf,

        #[arg(short = 'c', long, default_value = "zlib")]
        compression: Option<Compression>
    },
    Expand {
        source: PathBuf,

        #[arg(short, long, default_value = ".")]
        output: PathBuf,

        #[arg(short = 'c', long, default_value = "zlib")]
        compression: Option<Compression>
    },
    Cache {
        #[command(subcommand)]
        sub_command: CacheSubCommand
    }
}

#[derive(Clone, Subcommand)]
enum CacheSubCommand {
    Clear,
    Show
}

fn canonicalize_dir(path: PathBuf) -> PathBuf {
    fs::create_dir_all(&path).expect("Failed to create directory");
    fs::canonicalize(path).expect("Failed to canonicalize path")
}

#[tokio::main]
async fn main() {
    match Cli::parse().subcommand {
        SubCommand::Pack {
            source,
            output,
            jetfuel_path,
            compression
        } => perform_pack(output, jetfuel_path, source, compression).await,

        SubCommand::Unpack {
            source,
            output,
            compression
        } => perform_unpack(source, canonicalize_dir(output), compression),
        
        SubCommand::Peek {
            file,
            compression
        } => perform_peek(file, compression).await,
        
        SubCommand::Expand {
            source,
            output,
            compression
        } => perform_expand(source, canonicalize_dir(output), compression).await,

        SubCommand::Cache { sub_command: CacheSubCommand::Show } => {
            println!("Jet cache directory is {}", cache_dir().to_str().unwrap());
        }

        SubCommand::Cache { sub_command: CacheSubCommand::Clear } => {
            print!("Really clear jet caches? [Y/N] -> ");
            let _ = stdout().flush();
            let mut slice = [0u8; 1];
            stdin().read_exact(&mut slice).expect("Failed to read character");

            match slice[0] {
                b'Y' => {}
                b'y' => {
                    println!("You must type an uppercase Y.");
                    return;
                }
                b'n' | b'N' => return,
                other => {
                    println!("Invalid character {}", char::from(other));
                    return;
                }
            }

            println!("Clearing caches...");
            fs::remove_dir_all(cache_dir())
                .expect("Failed to delete cache directory");
            println!("Successfully cleared caches.");
        }
    }
}

fn parse_compression<P : AsRef<Path>>(compression: Option<Compression>, source: P) -> Compression {
    compression.unwrap_or_else(|| {
        let Some(ext) = source.as_ref().extension() else {
            return Compression::None;
        };

        match ext.to_str().unwrap() {
            jp::EXTENSION => Compression::None,
            jp_zlib::EXTENSION => Compression::Zlib,
            extension => {
                println!("{}: unknown compression of source file (extension: {}); assuming none", "warning".yellow(), extension);
                Compression::None
            }
        }
    })
}

async fn perform_pack(output: PathBuf, jetfuel_path: Option<PathBuf>, source: PathBuf, compression: Compression) {
    let mut writer = std::fs::File::create(&output)
        .expect(&format!("Failed to create file: {:?}", &output));
    let jetfuel_path = jetfuel_path.unwrap_or_else(|| source.join("jetfuel.xml"));
            
    let jetfuel_reader = std::io::BufReader::new(
        std::fs::File::open(&jetfuel_path)
            .expect(&format!("Failed to open path: {:?} (does it exist?)", &jetfuel_path))
    );
            
    let jetfuel: SourceManifest = quick_xml::de::from_reader(jetfuel_reader)
        .expect(&format!("Failed to read contents of {:?}", jetfuel_path));
            
    match compression {
        Compression::None => jp::pack(&mut writer, Some(jetfuel_path), jetfuel, source).await,
        Compression::Zlib => jp_zlib::pack(&mut writer, Some(jetfuel_path), jetfuel, source).await,
    }
}

fn perform_unpack(source: PathBuf, output: PathBuf, compression: Option<Compression>) {
    let reader = std::fs::File::open(&source)
                .expect(&format!("Failed to open file: {:?}", &source));

    match parse_compression(compression, &source) {
        Compression::None => jp::unpack(reader, output),
        Compression::Zlib => jp_zlib::unpack(reader, output)
    }
}

async fn perform_peek(source: PathBuf, compression: Option<Compression>) {
    let ps = SyntaxSet::load_defaults_newlines();
    let ts = ThemeSet::load_defaults();
    
    let syntax = ps.find_syntax_by_extension("xml").unwrap();
    let mut h = HighlightLines::new(syntax, &ts.themes["base16-ocean.dark"]);
    
    let reader = std::fs::File::open(&source)
                .expect(&format!("Failed to open file: {:?}", &source));

    let contents = match parse_compression(compression, &source) {
        Compression::None => jp::unpack_selective(reader, "@jetfuel.xml"),
        Compression::Zlib => jp_zlib::unpack_selective(reader, "@jetfuel.xml")
    };
    
    match contents {
        Some(contents) => {
            let contents = match String::from_utf8(contents) {
                Ok(contents) => contents,
                Err(err) => {
                    eprintln!("{}: failed to read jetfuel.xml manifest as UTF-8: {}", "error".red(), err);
                    return;
                },
            };
            
            for line in LinesWithEndings::from(&contents) {
                let ranges: Vec<(Style, &str)> = h.highlight_line(line, &ps).unwrap();
                let escaped = as_24_bit_terminal_escaped(&ranges[..], false);
                print!("{}", escaped);
            }
        },
        None => {
            eprintln!("{}: no @jetfuel.xml file is present; cannot peek!", "error".red());
        }
    }
}

async fn perform_expand(source: PathBuf, output: PathBuf, compression: Option<Compression>) {
    let reader = std::fs::File::open(&source)
                .expect(&format!("Failed to open file: {:?}", &source));

    match parse_compression(compression, &source) {
        Compression::None => jp::expand(reader, output).await,
        Compression::Zlib => jp_zlib::expand(reader, output).await
    }
}
