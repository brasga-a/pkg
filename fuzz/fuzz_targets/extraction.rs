#![no_main]

use std::io::Write;

use libfuzzer_sys::fuzz_target;
use pkg_core::format::{self, ExtractionLimits};
use tempfile::{Builder, TempDir};

fuzz_target!(|data: &[u8]| {
    if data.len() > 2 * 1024 * 1024 {
        return;
    }
    let suffix = match data.first().copied().unwrap_or_default() % 3 {
        0 => ".deb",
        1 => ".rpm",
        _ => ".pkg.tar.zst",
    };
    let Ok(mut file) = Builder::new().suffix(suffix).tempfile() else {
        return;
    };
    if file.write_all(data).is_err() {
        return;
    }
    let Ok(format) = format::detect_format(file.path()) else {
        return;
    };
    let adapter = format::get_adapter(format);
    let Ok(destination) = TempDir::new() else {
        return;
    };
    let limits = ExtractionLimits {
        max_entries: 256,
        max_total_bytes: 4 * 1024 * 1024,
        max_single_file_bytes: 1024 * 1024,
    };
    let _ = adapter.extract_payload(file.path(), destination.path(), &limits);
});
