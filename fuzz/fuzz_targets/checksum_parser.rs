#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = mvs_manager::install_release::expected_sha256_from_checksums(s, "x.tar.gz");
        if s.len() >= 64 {
            let _ = mvs_manager::install_release::parse_hex_sha256(&s[..64]);
        }
    }
});
