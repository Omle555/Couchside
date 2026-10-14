//! HTTP `Range` header parsing for seeking in video streams.

/// The part of a file to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRange {
    /// Send the whole file with `200 OK`.
    Full,
    /// Send bytes `start..=end` with `206 Partial Content`.
    Partial {
        /// First byte (inclusive).
        start: u64,
        /// Last byte (inclusive).
        end: u64,
    },
    /// Reply `416 Range Not Satisfiable`.
    Unsatisfiable,
}

/// Parses a `Range: bytes=...` header for a file of `len` bytes.
/// Only the first range of a multi-range request is honoured; malformed
/// headers are ignored (whole file), as RFC 9110 allows.
pub fn parse_range(header: Option<&str>, len: u64) -> ByteRange {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return ByteRange::Full;
    };
    let first = spec.split(',').next().unwrap_or("").trim();
    let Some((start_text, end_text)) = first.split_once('-') else {
        return ByteRange::Full;
    };
    let (start_text, end_text) = (start_text.trim(), end_text.trim());

    if start_text.is_empty() {
        // Suffix range: the last N bytes.
        let Ok(suffix) = end_text.parse::<u64>() else {
            return ByteRange::Full;
        };
        if suffix == 0 || len == 0 {
            return ByteRange::Unsatisfiable;
        }
        return ByteRange::Partial {
            start: len.saturating_sub(suffix),
            end: len - 1,
        };
    }

    let Ok(start) = start_text.parse::<u64>() else {
        return ByteRange::Full;
    };
    if start >= len {
        return ByteRange::Unsatisfiable;
    }
    let end = if end_text.is_empty() {
        len - 1
    } else {
        match end_text.parse::<u64>() {
            Ok(end) if end >= start => end.min(len - 1),
            _ => return ByteRange::Full,
        }
    };
    ByteRange::Partial { start, end }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_header_means_full() {
        assert_eq!(parse_range(None, 100), ByteRange::Full);
        assert_eq!(parse_range(Some("items=0-1"), 100), ByteRange::Full);
        assert_eq!(parse_range(Some("bytes=abc"), 100), ByteRange::Full);
    }

    #[test]
    fn open_ended() {
        assert_eq!(
            parse_range(Some("bytes=0-"), 100),
            ByteRange::Partial { start: 0, end: 99 }
        );
        assert_eq!(
            parse_range(Some("bytes=50-"), 100),
            ByteRange::Partial { start: 50, end: 99 }
        );
    }

    #[test]
    fn bounded_and_clamped() {
        assert_eq!(
            parse_range(Some("bytes=10-19"), 100),
            ByteRange::Partial { start: 10, end: 19 }
        );
        assert_eq!(
            parse_range(Some("bytes=10-1000"), 100),
            ByteRange::Partial { start: 10, end: 99 }
        );
        assert_eq!(parse_range(Some("bytes=20-10"), 100), ByteRange::Full);
    }

    #[test]
    fn suffix() {
        assert_eq!(
            parse_range(Some("bytes=-10"), 100),
            ByteRange::Partial { start: 90, end: 99 }
        );
        assert_eq!(
            parse_range(Some("bytes=-500"), 100),
            ByteRange::Partial { start: 0, end: 99 }
        );
        assert_eq!(parse_range(Some("bytes=-0"), 100), ByteRange::Unsatisfiable);
    }

    #[test]
    fn beyond_end() {
        assert_eq!(
            parse_range(Some("bytes=100-"), 100),
            ByteRange::Unsatisfiable
        );
        assert_eq!(parse_range(Some("bytes=0-"), 0), ByteRange::Unsatisfiable);
    }

    #[test]
    fn multi_range_uses_first() {
        assert_eq!(
            parse_range(Some("bytes=0-9, 20-29"), 100),
            ByteRange::Partial { start: 0, end: 9 }
        );
    }
}
