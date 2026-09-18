#![no_main]

use std::io::Write;

use libfuzzer_sys::fuzz_target;
use pkg_core::format;
use tempfile::Builder;

fuzz_target!(|data: &[u8]| {
    // Keep malformed inputs bounded so the target remains useful in CI and
    // cannot turn a corpus entry into an unbounded allocation request.
    if data.len() > 8 * 1024 * 1024 {
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
    let _ = adapter.parse_metadata(file.path());
});
