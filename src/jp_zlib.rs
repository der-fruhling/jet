use std::{io::{Write, Read}, path::Path};

use libflate::zlib::{Encoder, Decoder};

use crate::jp::{self, SourceManifest};

pub const EXTENSION: &'static str = "jpz";

pub async fn pack<W : Write, P1 : AsRef<Path>, P2 : AsRef<Path>>(writer: W, manifest_path: Option<P1>, manifest: SourceManifest, source_dir: P2) {
    let mut encoder = Encoder::new(writer)
        .expect("Failed to setup ZLIB encoder");
    
    jp::pack(&mut encoder, manifest_path, manifest, source_dir).await;
    
    encoder.finish().into_result()
        .expect("Failed to finish ZLIB encoding");
}

pub fn unpack<R : Read, P : AsRef<Path>>(reader: R, target_dir: P) {
    let decoder = Decoder::new(reader)
        .expect("Failed to setup ZLIB decoder");
    
    jp::unpack(decoder, target_dir);
}

pub fn unpack_selective<R : Read>(reader: R, name: &str) -> Option<Vec<u8>> {
    let decoder = Decoder::new(reader)
        .expect("Failed to setup ZLIB decoder");

    jp::unpack_selective(decoder, name)
}

pub async fn expand<R : Read, P : AsRef<Path>>(reader: R, target_dir: P) {
    let decoder = Decoder::new(reader)
        .expect("Failed to setup ZLIB decoder");
    
    jp::expand(decoder, target_dir).await;
}
