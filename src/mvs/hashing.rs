// SPDX-License-Identifier: AGPL-3.0-only
use std::{fs, path::Path};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn hash_items<I, S>(items: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut values: Vec<String> = items
        .into_iter()
        .map(|item| item.as_ref().to_owned())
        .collect();
    values.sort();

    let joined = values.join("\n");
    sha256_hex(joined.as_bytes())
}

pub fn hash_file(path: &Path) -> Result<String> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read file: {}", path.display()))?;
    // Normalize CRLF→LF so AI schema hashes stay stable under Git autocrlf on Windows.
    Ok(sha256_hex(&strip_carriage_returns(&bytes)))
}

fn strip_carriage_returns(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().copied().filter(|&b| b != b'\r').collect()
}

#[cfg(test)]
mod tests {
    use super::{hash_file, sha256_hex, strip_carriage_returns};
    use std::io::Write;

    #[test]
    fn hash_file_ignores_crlf_vs_lf() {
        let dir = std::env::temp_dir().join(format!(
            "mvs-hash-crlf-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let lf = dir.join("lf.json");
        let crlf = dir.join("crlf.json");
        std::fs::write(&lf, b"{\"a\":1}\n").expect("write lf");
        {
            let mut f = std::fs::File::create(&crlf).expect("create crlf");
            f.write_all(b"{\"a\":1}\r\n").expect("write crlf");
        }
        assert_eq!(
            hash_file(&lf).expect("hash lf"),
            hash_file(&crlf).expect("hash crlf")
        );
        assert_eq!(
            sha256_hex(b"{\"a\":1}\n"),
            sha256_hex(&strip_carriage_returns(b"{\"a\":1}\r\n"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
