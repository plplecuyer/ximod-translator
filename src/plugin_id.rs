//! The identity of a plugin (B-12): name, size and CRC-32 of the file as it
//! was opened, written in the header of its mod's dictionary so a later
//! opening recognises the version the dictionary was made for.

use std::io::Read;
use std::path::Path;

/// Name, size and CRC-32 of a file; `written`: Unix seconds of the write of
/// the dictionary that carries it (0 when unknown).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ident {
    pub plugin: String,
    pub size: u64,
    pub crc: u32,
    pub written: i64,
}

impl Ident {
    /// The same file content: size and CRC-32 (the name may differ).
    pub fn same_content(&self, other: &Ident) -> bool {
        self.size == other.size && self.crc == other.crc
    }

    /// The CRC-32 as eight upper-case hexadecimal digits.
    pub fn crc_hex(&self) -> String {
        format!("{:08X}", self.crc)
    }
}

/// The identity of a file, read in 1 MiB blocks.
pub fn of_file(path: &Path) -> std::io::Result<Ident> {
    let mut f = std::fs::File::open(path)?;
    let mut h = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        size += n as u64;
    }
    Ok(Ident {
        plugin: path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        size,
        crc: h.finalize(),
        written: 0,
    })
}

/// Parse an eight-digit hexadecimal CRC.
pub fn parse_crc(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim(), 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_a_file() {
        let p = std::env::temp_dir().join(format!("ximod-crc-{}.esp", std::process::id()));
        std::fs::write(&p, b"123456789").unwrap();
        let id = of_file(&p).unwrap();
        // The check value of CRC-32/ISO-HDLC.
        assert_eq!(id.crc, 0xCBF4_3926);
        assert_eq!(id.crc_hex(), "CBF43926");
        assert_eq!(id.size, 9);
        assert!(id.plugin.ends_with(".esp"));
        assert_eq!(parse_crc("cbf43926"), Some(0xCBF4_3926));
        let _ = std::fs::remove_file(p);
    }
}
