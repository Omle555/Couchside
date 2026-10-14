//! Video encoder settings for full transcodes: H.264 8-bit on the NVIDIA GPU (NVENC) when it
//! is switched on and works, otherwise on the processor with x264 (rule 7.5).

use crate::process::new_command;
use crate::segments::SEGMENT_SECONDS;
use std::path::Path;
use std::time::Duration;

/// Longest the NVENC self-test may take.
const SELF_TEST_TIMEOUT: Duration = Duration::from_secs(20);

/// How a re-encoded video is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoEncode {
    /// Height to scale down to, when the source is taller than the viewer wants.
    pub height: Option<i64>,
    /// Height of the source (picks the bitrate cap when the video isn't scaled).
    pub source_height: Option<i64>,
    /// The source is HDR: tone-map it to SDR, which is all H.264 8-bit can show.
    pub hdr: Option<Hdr>,
}

/// The kind of HDR a video uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hdr {
    /// HDR10 / Dolby Vision base layer (SMPTE ST 2084).
    Pq,
    /// Hybrid log-gamma (broadcast HDR).
    Hlg,
}

impl Hdr {
    /// zimg's name for the transfer.
    fn transfer(self) -> &'static str {
        match self {
            Self::Pq => "smpte2084",
            Self::Hlg => "arib-std-b67",
        }
    }
}

impl VideoEncode {
    /// Arguments before `-i`: hardware decoding on the GPU path. HDR goes through Vulkan so
    /// libplacebo can tone-map on the GPU (several times faster than real time at 4K, where
    /// the processor can't keep up); anything else decodes with CUDA. ffmpeg decodes in
    /// software by itself where the GPU can't (10-bit H.264, old formats).
    pub(crate) fn input_args(&self, gpu: bool) -> Vec<String> {
        let args: &[&str] = match (gpu, self.hdr) {
            (false, _) => &[],
            (true, Some(_)) => &[
                "-init_hw_device",
                "vulkan",
                "-hwaccel",
                "vulkan",
                "-hwaccel_output_format",
                "vulkan",
            ],
            (true, None) => &["-hwaccel", "cuda"],
        };
        args.iter().map(|a| (*a).to_string()).collect()
    }

    /// Highest bitrate in kb/s for the output height (LAN only, so generous).
    fn max_kbps(&self) -> u32 {
        match self.height.or(self.source_height).unwrap_or(1080) {
            h if h > 1080 => 20_000,
            h if h > 720 => 10_000,
            h if h > 480 => 5_000,
            _ => 2_000,
        }
    }

    /// The `-vf` chain: scale (keeping both sides even, as 4:2:0 needs), tone-map HDR, and end
    /// in 8-bit 4:2:0. HDR on the GPU is one libplacebo pass; on the processor it is zscale
    /// and tonemap at the smaller size.
    fn filters(&self, gpu: bool) -> String {
        if let (true, Some(_)) = (gpu, self.hdr) {
            let size = self
                .height
                .map_or(String::new(), |h| format!("w=-2:h={h}:"));
            return format!(
                "libplacebo={size}tonemapping=hable:colorspace=bt709:color_primaries=bt709:\
                 color_trc=bt709:range=tv:format=yuv420p"
            );
        }
        let mut chain = vec![match self.height {
            Some(h) => format!("scale=-2:{h}"),
            None => "scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string(),
        }];
        if let Some(hdr) = self.hdr {
            chain.push(format!(
                "zscale=tin={}:min=bt2020nc:pin=bt2020:t=linear:npl=100,format=gbrpf32le,\
                 zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv",
                hdr.transfer()
            ));
        }
        chain.push("format=yuv420p".to_string());
        chain.join(",")
    }

    /// Filter and encoder arguments, for the GPU or the processor. Keyframes fall only on
    /// segment boundaries (no scene-cut or GOP keyframes), so every segment is
    /// [`SEGMENT_SECONDS`] long.
    pub(crate) fn args(&self, gpu: bool) -> Vec<String> {
        let max = self.max_kbps();
        let mut args = vec!["-vf".to_string(), self.filters(gpu)];
        if gpu {
            args.extend(
                [
                    "-c:v",
                    "h264_nvenc",
                    "-preset",
                    "p5",
                    "-tune",
                    "hq",
                    "-rc",
                    "vbr",
                ]
                .map(String::from),
            );
            args.extend(["-b:v".into(), format!("{}k", max * 7 / 10)]);
            args.extend(["-no-scenecut", "1"].map(String::from));
        } else {
            args.extend(
                [
                    "-c:v",
                    "libx264",
                    "-preset",
                    "veryfast",
                    "-crf",
                    "21",
                    "-sc_threshold",
                    "0",
                ]
                .map(String::from),
            );
        }
        args.extend([
            "-maxrate".into(),
            format!("{max}k"),
            "-bufsize".into(),
            format!("{}k", max * 2),
            "-g".into(),
            "9999".into(),
            "-forced-idr".into(),
            "1".into(),
            "-profile:v".into(),
            "high".into(),
            "-force_key_frames".into(),
            // `t` counts from where this ffmpeg started, which is always a segment start.
            format!("expr:gte(t,n_forced*{SEGMENT_SECONDS})"),
        ]);
        args
    }
}

/// Whether ffmpeg can encode with NVENC on this PC (driver present, card free).
pub(crate) async fn nvenc_works(ffmpeg: &Path) -> bool {
    let mut cmd = new_command(ffmpeg);
    cmd.args([
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=black:s=256x256:d=1",
        "-c:v",
        "h264_nvenc",
        "-f",
        "null",
        "-",
    ]);
    match tokio::time::timeout(SELF_TEST_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) if out.status.success() => true,
        Ok(Ok(out)) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            tracing::warn!(error = %stderr.trim(), "NVIDIA encoder test failed");
            false
        }
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "could not run the NVIDIA encoder test");
            false
        }
        Err(_) => {
            tracing::warn!("NVIDIA encoder test timed out");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(height: Option<i64>, source: i64, hdr: Option<Hdr>) -> VideoEncode {
        VideoEncode {
            height,
            source_height: Some(source),
            hdr,
        }
    }

    fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let i = args.iter().position(|a| a == flag)?;
        args.get(i + 1).map(String::as_str)
    }

    #[test]
    fn bitrate_follows_output_height() {
        assert_eq!(encode(None, 2160, None).max_kbps(), 20_000);
        assert_eq!(encode(Some(1080), 2160, None).max_kbps(), 10_000);
        assert_eq!(encode(Some(720), 1080, None).max_kbps(), 5_000);
        assert_eq!(encode(None, 480, None).max_kbps(), 2_000);
    }

    #[test]
    fn filters_scale_then_tone_map() {
        assert_eq!(
            encode(Some(720), 1080, None).filters(true),
            "scale=-2:720,format=yuv420p"
        );
        assert!(encode(None, 1080, None)
            .filters(false)
            .starts_with("scale=trunc("));
        let cpu_hdr = encode(Some(1080), 2160, Some(Hdr::Pq)).filters(false);
        assert!(cpu_hdr.starts_with("scale=-2:1080,zscale=tin=smpte2084:"));
        assert!(cpu_hdr.contains("tonemap=tonemap=hable"));
        assert!(cpu_hdr.ends_with("format=yuv420p"));
        let gpu_hdr = encode(Some(1080), 2160, Some(Hdr::Hlg)).filters(true);
        assert!(gpu_hdr.starts_with("libplacebo=w=-2:h=1080:tonemapping=hable:"));
        assert!(encode(None, 2160, Some(Hdr::Pq))
            .filters(true)
            .starts_with("libplacebo=tonemapping="));
    }

    #[test]
    fn hdr_decodes_through_vulkan_on_the_gpu() {
        let hdr = encode(None, 2160, Some(Hdr::Pq));
        assert_eq!(
            value_after(&hdr.input_args(true), "-hwaccel"),
            Some("vulkan")
        );
        assert!(hdr.input_args(false).is_empty());
        let sdr = encode(None, 1080, None);
        assert_eq!(value_after(&sdr.input_args(true), "-hwaccel"), Some("cuda"));
    }

    #[test]
    fn gpu_and_cpu_arguments() {
        let gpu = encode(Some(1080), 2160, None).args(true);
        assert_eq!(value_after(&gpu, "-c:v"), Some("h264_nvenc"));
        assert_eq!(value_after(&gpu, "-b:v"), Some("7000k"));
        assert_eq!(value_after(&gpu, "-maxrate"), Some("10000k"));
        assert_eq!(
            value_after(&gpu, "-force_key_frames"),
            Some("expr:gte(t,n_forced*6)")
        );
        let cpu = encode(Some(720), 1080, None).args(false);
        assert_eq!(value_after(&cpu, "-c:v"), Some("libx264"));
        assert_eq!(value_after(&cpu, "-sc_threshold"), Some("0"));
        assert!(!cpu.iter().any(|a| a == "-no-scenecut"));
    }
}
