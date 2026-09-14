//! Ecosystem-specific version ordering preserving source distribution semantics (INV-008).
//!
//! Does not coerce versions into SemVer, honoring native distribution version syntax:
//! - Debian: epochs, upstream version with tildes, debian revisions.
//! - RPM: rpmvercmp algorithm with segment classification, tildes, and carets.
//! - ALPM: Arch Linux vercmp with epochs, pkgver, and pkgrel.

use std::cmp::Ordering;

/// Distro ecosystem version ordering evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VersionEcosystem {
    Debian,
    Rpm,
    Alpm,
}

/// Compares two version strings according to the specified distribution ecosystem rules.
pub fn compare_versions(a: &str, b: &str, ecosystem: VersionEcosystem) -> Ordering {
    match ecosystem {
        VersionEcosystem::Debian => compare_debian_versions(a, b),
        VersionEcosystem::Rpm => compare_rpm_versions(a, b),
        VersionEcosystem::Alpm => compare_alpm_versions(a, b),
    }
}

// -----------------------------------------------------------------------------
// Debian Version Comparison
// -----------------------------------------------------------------------------

fn parse_debian_version(v: &str) -> (u64, &str, &str) {
    let (epoch, rest) = if let Some(colon) = v.find(':') {
        let epoch_str = &v[..colon];
        (epoch_str.parse::<u64>().unwrap_or(0), &v[colon + 1..])
    } else {
        (0, v)
    };

    let (upstream, revision) = if let Some(hyphen) = rest.rfind('-') {
        (&rest[..hyphen], &rest[hyphen + 1..])
    } else {
        (rest, "")
    };

    (epoch, upstream, revision)
}

fn compare_debian_string_part(a: &str, b: &str) -> Ordering {
    let mut a_chars = a.chars().peekable();
    let mut b_chars = b.chars().peekable();

    fn char_order(c: Option<char>) -> i32 {
        match c {
            None => 0,
            Some('~') => -1,
            Some(ch) if ch.is_ascii_alphabetic() => ch as i32,
            Some(ch) => (ch as i32) + 256,
        }
    }

    while a_chars.peek().is_some() || b_chars.peek().is_some() {
        // Compare non-digits first
        while (a_chars.peek().is_some() && !a_chars.peek().unwrap().is_ascii_digit())
            || (b_chars.peek().is_some() && !b_chars.peek().unwrap().is_ascii_digit())
        {
            let ca = a_chars.next();
            let cb = b_chars.next();
            let oa = char_order(ca);
            let ob = char_order(cb);
            if oa != ob {
                return oa.cmp(&ob);
            }
        }

        // Compare contiguous digit blocks
        let mut a_num_str = String::new();
        while let Some(&ch) = a_chars.peek() {
            if ch.is_ascii_digit() {
                a_num_str.push(ch);
                a_chars.next();
            } else {
                break;
            }
        }

        let mut b_num_str = String::new();
        while let Some(&ch) = b_chars.peek() {
            if ch.is_ascii_digit() {
                b_num_str.push(ch);
                b_chars.next();
            } else {
                break;
            }
        }

        if !a_num_str.is_empty() || !b_num_str.is_empty() {
            let na: u64 = a_num_str.parse().unwrap_or(0);
            let nb: u64 = b_num_str.parse().unwrap_or(0);
            if na != nb {
                return na.cmp(&nb);
            }
        }
    }

    Ordering::Equal
}

/// Compares two Debian version strings (e.g., `2:1.2.3~rc1-1ubuntu2`).
pub fn compare_debian_versions(a: &str, b: &str) -> Ordering {
    let (epoch_a, up_a, rev_a) = parse_debian_version(a);
    let (epoch_b, up_b, rev_b) = parse_debian_version(b);

    if epoch_a != epoch_b {
        return epoch_a.cmp(&epoch_b);
    }

    let up_cmp = compare_debian_string_part(up_a, up_b);
    if up_cmp != Ordering::Equal {
        return up_cmp;
    }

    compare_debian_string_part(rev_a, rev_b)
}

// -----------------------------------------------------------------------------
// RPM Version Comparison (rpmvercmp)
// -----------------------------------------------------------------------------

fn parse_rpm_version(v: &str) -> (u64, &str, &str) {
    let (epoch, rest) = if let Some(colon) = v.find(':') {
        let epoch_str = &v[..colon];
        (epoch_str.parse::<u64>().unwrap_or(0), &v[colon + 1..])
    } else {
        (0, v)
    };

    let (version, release) = if let Some(hyphen) = rest.rfind('-') {
        (&rest[..hyphen], &rest[hyphen + 1..])
    } else {
        (rest, "")
    };

    (epoch, version, release)
}

/// Implements standard RPM `rpmvercmp` algorithm.
pub fn rpmvercmp(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }

    let mut a_bytes = a.as_bytes();
    let mut b_bytes = b.as_bytes();

    while !a_bytes.is_empty() || !b_bytes.is_empty() {
        // Trim non-alphanumeric, non-tilde, non-caret prefixes
        let trim_a = a_bytes
            .iter()
            .position(|&b| b.is_ascii_alphanumeric() || b == b'~' || b == b'^')
            .unwrap_or(a_bytes.len());
        a_bytes = &a_bytes[trim_a..];
        let trim_b = b_bytes
            .iter()
            .position(|&b| b.is_ascii_alphanumeric() || b == b'~' || b == b'^')
            .unwrap_or(b_bytes.len());
        b_bytes = &b_bytes[trim_b..];

        // Handle tildes: '~' sorts earlier than anything (including end-of-string)
        let a_tilde = a_bytes.first() == Some(&b'~');
        let b_tilde = b_bytes.first() == Some(&b'~');
        if a_tilde || b_tilde {
            if a_tilde && !b_tilde {
                return Ordering::Less;
            }
            if !a_tilde && b_tilde {
                return Ordering::Greater;
            }
            a_bytes = &a_bytes[1..];
            b_bytes = &b_bytes[1..];
            continue;
        }

        // Handle carets: '^' sorts earlier than anything EXCEPT tilde and end-of-string
        let a_caret = a_bytes.first() == Some(&b'^');
        let b_caret = b_bytes.first() == Some(&b'^');
        if a_caret || b_caret {
            if a_bytes.is_empty() {
                return Ordering::Less;
            }
            if b_bytes.is_empty() {
                return Ordering::Greater;
            }
            if a_caret && !b_caret {
                return Ordering::Less;
            }
            if !a_caret && b_caret {
                return Ordering::Greater;
            }
            a_bytes = &a_bytes[1..];
            b_bytes = &b_bytes[1..];
            continue;
        }

        if a_bytes.is_empty() {
            return Ordering::Less;
        }
        if b_bytes.is_empty() {
            return Ordering::Greater;
        }

        // Segment classification: digits vs letters
        let is_digit = a_bytes[0].is_ascii_digit();
        if is_digit != b_bytes[0].is_ascii_digit() {
            return if is_digit {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }

        let end_a = a_bytes
            .iter()
            .position(|&b| {
                if is_digit {
                    !b.is_ascii_digit()
                } else {
                    !b.is_ascii_alphabetic()
                }
            })
            .unwrap_or(a_bytes.len());
        let end_b = b_bytes
            .iter()
            .position(|&b| {
                if is_digit {
                    !b.is_ascii_digit()
                } else {
                    !b.is_ascii_alphabetic()
                }
            })
            .unwrap_or(b_bytes.len());

        let seg_a = std::str::from_utf8(&a_bytes[..end_a]).unwrap_or("");
        let seg_b = std::str::from_utf8(&b_bytes[..end_b]).unwrap_or("");

        a_bytes = &a_bytes[end_a..];
        b_bytes = &b_bytes[end_b..];

        if is_digit {
            // Trim leading zeros
            let trimmed_a = seg_a.trim_start_matches('0');
            let trimmed_b = seg_b.trim_start_matches('0');
            if trimmed_a.len() != trimmed_b.len() {
                return trimmed_a.len().cmp(&trimmed_b.len());
            }
            let num_cmp = trimmed_a.cmp(trimmed_b);
            if num_cmp != Ordering::Equal {
                return num_cmp;
            }
        } else {
            let str_cmp = seg_a.cmp(seg_b);
            if str_cmp != Ordering::Equal {
                return str_cmp;
            }
        }
    }

    Ordering::Equal
}

/// Compares two RPM version strings (accounting for epoch, version, release).
pub fn compare_rpm_versions(a: &str, b: &str) -> Ordering {
    let (epoch_a, ver_a, rel_a) = parse_rpm_version(a);
    let (epoch_b, ver_b, rel_b) = parse_rpm_version(b);

    if epoch_a != epoch_b {
        return epoch_a.cmp(&epoch_b);
    }

    let ver_cmp = rpmvercmp(ver_a, ver_b);
    if ver_cmp != Ordering::Equal {
        return ver_cmp;
    }

    rpmvercmp(rel_a, rel_b)
}

// -----------------------------------------------------------------------------
// ALPM Version Comparison (vercmp)
// -----------------------------------------------------------------------------

/// Implements Arch Linux ALPM `vercmp` algorithm.
pub fn compare_alpm_versions(a: &str, b: &str) -> Ordering {
    // In ALPM: [epoch:]pkgver[-pkgrel]
    let (epoch_a, rest_a) = if let Some(colon) = a.find(':') {
        (a[..colon].parse::<u64>().unwrap_or(0), &a[colon + 1..])
    } else {
        (0, a)
    };

    let (epoch_b, rest_b) = if let Some(colon) = b.find(':') {
        (b[..colon].parse::<u64>().unwrap_or(0), &b[colon + 1..])
    } else {
        (0, b)
    };

    if epoch_a != epoch_b {
        return epoch_a.cmp(&epoch_b);
    }

    let (ver_a, rel_a) = if let Some(hyphen) = rest_a.rfind('-') {
        (&rest_a[..hyphen], &rest_a[hyphen + 1..])
    } else {
        (rest_a, "")
    };

    let (ver_b, rel_b) = if let Some(hyphen) = rest_b.rfind('-') {
        (&rest_b[..hyphen], &rest_b[hyphen + 1..])
    } else {
        (rest_b, "")
    };

    let ver_cmp = rpmvercmp(ver_a, ver_b);
    if ver_cmp != Ordering::Equal {
        return ver_cmp;
    }

    rpmvercmp(rel_a, rel_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debian_version_ordering() {
        assert_eq!(compare_debian_versions("1.0~rc1", "1.0"), Ordering::Less);
        assert_eq!(compare_debian_versions("1.0", "1.0-1"), Ordering::Less);
        assert_eq!(compare_debian_versions("1.0-1", "1.0-2"), Ordering::Less);
        assert_eq!(compare_debian_versions("1.0-2", "1.1-1"), Ordering::Less);
        assert_eq!(compare_debian_versions("1:1.0", "2.0"), Ordering::Greater);
    }

    #[test]
    fn test_rpm_version_ordering() {
        assert_eq!(compare_rpm_versions("1.0~rc1", "1.0"), Ordering::Less);
        assert_eq!(compare_rpm_versions("1.0", "1.0^20240101"), Ordering::Less);
        assert_eq!(
            compare_rpm_versions("1.0^20240101", "1.0.1"),
            Ordering::Less
        );
        assert_eq!(
            compare_rpm_versions("1.0-1.fc40", "1.0-2.fc40"),
            Ordering::Less
        );
        assert_eq!(
            compare_rpm_versions("2:1.0-1", "1:2.0-1"),
            Ordering::Greater
        );
    }

    #[test]
    fn test_alpm_version_ordering() {
        assert_eq!(compare_alpm_versions("1.0-1", "1.0-2"), Ordering::Less);
        assert_eq!(compare_alpm_versions("1.0-2", "1.1-1"), Ordering::Less);
        assert_eq!(compare_alpm_versions("1:1.0-1", "2.0-1"), Ordering::Greater);
    }
}
