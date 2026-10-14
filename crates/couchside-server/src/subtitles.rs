//! Conversion of external subtitle files to WebVTT, the only format browsers play.

/// Converts subtitle text in `format` (`srt`, `vtt`, `ass`, `ssa`) to WebVTT.
pub fn to_vtt(format: &str, input: &str) -> Option<String> {
    let text = input
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    match format {
        "vtt" => Some(text),
        "srt" => Some(srt_to_vtt(&text)),
        "ass" | "ssa" => Some(ass_to_vtt(&text)),
        _ => None,
    }
}

/// SRT and VTT differ mainly in the header and the millisecond separator.
fn srt_to_vtt(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    out.push_str("WEBVTT\n\n");
    for line in text.lines() {
        if line.contains("-->") {
            out.push_str(&line.replace(',', "."));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Converts `H:MM:SS.cc` (centiseconds) to `HH:MM:SS.mmm`.
fn ass_time(t: &str) -> Option<String> {
    let mut parts = t.trim().split(':');
    let h: u32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let sec_part = parts.next()?;
    let (s, frac) = sec_part.split_once('.').unwrap_or((sec_part, "0"));
    let s: u32 = s.parse().ok()?;
    let frac_digits: String = frac.chars().take(3).collect();
    let mut ms: u32 = frac_digits.parse().ok()?;
    for _ in frac_digits.len()..3 {
        ms *= 10;
    }
    Some(format!("{h:02}:{m:02}:{s:02}.{ms:03}"))
}

/// Removes `{\override}` tags and converts ASS line breaks.
fn ass_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '{' => in_tag = true,
            '}' if in_tag => in_tag = false,
            _ if in_tag => {}
            _ => out.push(c),
        }
    }
    out.replace("\\N", "\n")
        .replace("\\n", "\n")
        .replace("\\h", " ")
}

/// Extracts dialogue lines from an ASS/SSA script (styling is dropped).
fn ass_to_vtt(text: &str) -> String {
    let mut out = String::from("WEBVTT\n\n");
    let mut in_events = false;
    let mut fields: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_events = trimmed.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(format) = trimmed.strip_prefix("Format:") {
            fields = format
                .split(',')
                .map(|f| f.trim().to_ascii_lowercase())
                .collect();
            continue;
        }
        let Some(dialogue) = trimmed.strip_prefix("Dialogue:") else {
            continue;
        };
        if fields.is_empty() {
            continue;
        }
        let values: Vec<&str> = dialogue.splitn(fields.len(), ',').collect();
        let get = |name: &str| {
            fields
                .iter()
                .position(|f| f == name)
                .and_then(|i| values.get(i))
                .copied()
        };
        let (Some(start), Some(end), Some(body)) = (
            get("start").and_then(ass_time),
            get("end").and_then(ass_time),
            get("text"),
        ) else {
            continue;
        };
        let body = ass_text(body);
        if body.trim().is_empty() {
            continue;
        }
        out.push_str(&format!("{start} --> {end}\n{}\n\n", body.trim()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_converts() {
        let srt = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,500\r\nHello, world\r\n\r\n";
        let vtt = to_vtt("srt", srt).expect("supported");
        assert!(vtt.starts_with("WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.500\nHello, world\n"));
    }

    #[test]
    fn ass_converts() {
        let ass = "[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,{\\i1}Hi,{\\i0} there\\Nfriend\n";
        let vtt = to_vtt("ass", ass).expect("supported");
        assert_eq!(
            vtt,
            "WEBVTT\n\n00:00:01.500 --> 00:00:03.000\nHi, there\nfriend\n\n"
        );
    }

    #[test]
    fn unknown_format() {
        assert!(to_vtt("sub", "x").is_none());
    }
}
