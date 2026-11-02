//! End-to-end: build **compressed** archives on disk with ba2's writer, reopen
//! them through our `Archive::open`, and verify listing + decompressed
//! extraction. This exercises the real file-path entry point and the
//! decompression codecs (LZ4 for BSA SSE, zlib for BA2) through our wrapper.

use ba2::prelude::*;
use ba2::{
    fo4::{
        Archive as Fo4Archive, ArchiveKey as Fo4Key, ArchiveOptions as Fo4Opts, Chunk,
        ChunkCompressionOptions,
    },
    tes4::{
        Archive as Tes4Archive, ArchiveKey as Tes4Key, ArchiveOptions as Tes4Opts, Directory,
        DirectoryKey, File as Tes4File, FileCompressionOptions, Version,
    },
};

use ximod_translator::archive::{Archive, Format};

#[test]
fn bsa_sse_compressed_roundtrip_on_disk() {
    // A payload large and repetitive enough to actually compress.
    let payload = b"The quick brown fox jumps over the lazy dog. ".repeat(64);

    let copts = FileCompressionOptions::builder()
        .version(Version::SSE)
        .build();
    let file = Tes4File::from_decompressed(payload.as_slice())
        .compress(&copts)
        .expect("compress");
    assert!(file.is_compressed());

    let dir: Directory = [(DirectoryKey::from(b"mymod.esp".as_slice()), file)]
        .into_iter()
        .collect();
    let archive: Tes4Archive = [(Tes4Key::from(b"".as_slice()), dir)].into_iter().collect();
    let opts = Tes4Opts::builder().version(Version::SSE).build();

    let dir = std::env::temp_dir().join("ximod_arc_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("compressed_sse.bsa");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        archive.write(&mut f, &opts).expect("write bsa");
    }

    let a = Archive::open(&path).expect("open bsa");
    assert_eq!(a.format(), Format::Bsa);
    let idx = a
        .entries()
        .iter()
        .position(|e| e.path == "mymod.esp")
        .unwrap();
    assert!(a.entries()[idx].compressed, "stored compressed");
    assert_eq!(a.extract(idx).unwrap(), payload);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn ba2_zip_compressed_roundtrip_on_disk() {
    let payload = b"STRINGS payload line with repeated content. ".repeat(64);

    let copts = ChunkCompressionOptions::builder().build();
    let chunk = Chunk::from_decompressed(payload.as_slice())
        .compress(&copts)
        .expect("compress chunk");
    assert!(chunk.is_compressed());
    let file: ba2::fo4::File = [chunk].into_iter().collect();
    let archive: Fo4Archive = [(
        Fo4Key::from(b"strings\\mymod_english.strings".as_slice()),
        file,
    )]
    .into_iter()
    .collect();
    let opts = Fo4Opts::builder().strings(true).build();

    let dir = std::env::temp_dir().join("ximod_arc_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("compressed.ba2");
    {
        let mut f = std::fs::File::create(&path).unwrap();
        archive.write(&mut f, &opts).expect("write ba2");
    }

    let a = Archive::open(&path).expect("open ba2");
    assert_eq!(a.format(), Format::Ba2);
    assert_eq!(a.entries().len(), 1);
    let e = &a.entries()[0];
    assert_eq!(e.path, "strings/mymod_english.strings");
    assert!(e.compressed);
    assert_eq!(a.extract(0).unwrap(), payload);

    let _ = std::fs::remove_file(&path);
}
