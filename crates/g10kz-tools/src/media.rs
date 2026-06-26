//! Media pre-processing pipeline.
//!
//! # Pipeline
//! - **Image**: download → base64 data URL → `Part::ImageUrl`
//! - **Video**: `ffmpeg` subprocess → frame PNGs → base64 Parts (adaptive count)
//! - **Audio**: download → transcription model via LLM provider

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use base64::Engine as _;
use tracing::{debug, warn};

use g10kz_llm::types::Part;

/// Result of media pre-processing: one or more [`Part`]s.
#[derive(Debug)]
pub struct MediaOutput {
    pub parts: Vec<Part>,
    /// Human-readable label prepended by the engine (e.g., `"[影像分析]"`).
    pub label: String,
}

// ─── Static HTTP clients ─────────────────────────────────────────────────────

static DOWNLOAD_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static TRANSCRIBE_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn download_client() -> &'static reqwest::Client {
    DOWNLOAD_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(4)
            .build()
            .expect("media download client build failed")
    })
}

fn transcribe_client() -> &'static reqwest::Client {
    TRANSCRIBE_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(4)
            .build()
            .expect("media transcribe client build failed")
    })
}

// ─── Image ────────────────────────────────────────────────────────────────────

/// Download an image from `url`, encode as base64 data URL.
///
/// Remote URLs are passed through directly (Discord CDN URLs are already
/// accessible to the LLM vision model).
pub async fn process_image(url: &str) -> anyhow::Result<MediaOutput> {
    // If it's already a plain https URL, pass it through — vision models
    // can fetch it natively.  Only encode when it's not a regular URL.
    if url.starts_with("https://") || url.starts_with("http://") {
        return Ok(MediaOutput {
            parts: vec![Part::ImageUrl {
                url: url.to_owned(),
            }],
            label: "[圖片]".into(),
        });
    }

    // Otherwise download and base64-encode.
    let bytes = download_bytes(url).await?;
    let mime = guess_mime(&bytes);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let data_url = format!("data:{};base64,{}", mime, b64);

    Ok(MediaOutput {
        parts: vec![Part::ImageUrl { url: data_url }],
        label: "[圖片]".into(),
    })
}

fn guess_mime(bytes: &[u8]) -> &'static str {
    match bytes {
        b if b.starts_with(b"\x89PNG") => "image/png",
        b if b.starts_with(b"\xff\xd8\xff") => "image/jpeg",
        b if b.starts_with(b"GIF8") => "image/gif",
        b if b.starts_with(b"RIFF") && bytes.len() > 8 && &bytes[8..12] == b"WEBP" => "image/webp",
        _ => "image/jpeg",
    }
}

// ─── Video ────────────────────────────────────────────────────────────────────

/// Extract frames from a video using `ffmpeg`.
///
/// Adaptive frame count: ≤60s → 4 frames; ≤180s → 8; >180s → 12.
pub async fn process_video(
    url: &str,
    duration_hint_secs: Option<f64>,
) -> anyhow::Result<MediaOutput> {
    let n_frames = match duration_hint_secs {
        Some(d) if d <= 60.0 => 4,
        Some(d) if d <= 180.0 => 8,
        _ => 12,
    };

    let tmp_dir = tempdir()?;
    let output_pattern = tmp_dir.join("frame_%03d.jpg");

    // Extract evenly-spaced frames with ffmpeg
    let status = tokio::process::Command::new("ffmpeg")
        .args([
            "-i",
            url,
            "-vf",
            &format!(
                "select='not(mod(n\\,{}))',setpts=N/FRAME_RATE/TB",
                frame_step_expr(url, n_frames)
            ),
            "-vframes",
            &n_frames.to_string(),
            "-q:v",
            "4",
            output_pattern.to_str().unwrap_or("."),
            "-y",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => anyhow::bail!("ffmpeg exited with status {s}"),
        Err(e) => anyhow::bail!("ffmpeg not found or failed to start: {e}"),
    }

    // Collect frames
    let mut parts = Vec::new();
    for i in 1..=n_frames {
        let frame_path = tmp_dir.join(format!("frame_{i:03}.jpg"));
        if !frame_path.exists() {
            break;
        }
        match tokio::fs::read(&frame_path).await {
            Ok(bytes) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                parts.push(Part::ImageUrl {
                    url: format!("data:image/jpeg;base64,{}", b64),
                });
            }
            Err(e) => warn!("failed to read frame {i}: {e}"),
        }
    }

    if parts.is_empty() {
        anyhow::bail!("ffmpeg produced no frames");
    }

    debug!(frames = parts.len(), "video frames extracted");
    Ok(MediaOutput {
        parts,
        label: "[影片分析]".into(),
    })
}

fn frame_step_expr(_url: &str, n_frames: usize) -> String {
    // Without knowing total frame count, use a fixed step heuristic
    // (30fps * expected_secs / n_frames).  Good enough for short videos.
    (30 * 60 / n_frames).to_string()
}

// ─── Audio ────────────────────────────────────────────────────────────────────

/// Download audio and transcribe via OpenAI Whisper-compatible endpoint.
///
/// Requires `provider_base_url` and `api_key`.  Returns plain text transcript.
pub async fn transcribe_audio(url: &str) -> anyhow::Result<String> {
    // Download audio file
    let bytes = download_bytes(url).await?;

    // Determine extension from URL
    let ext = url.rsplit('.').next().unwrap_or("mp3");
    let filename = format!("audio.{ext}");

    // POST to /audio/transcriptions (OpenAI-compatible)
    // NOTE: Requires LLM provider to expose whisper endpoint.
    // This is a best-effort implementation — errors are surfaced to the engine.
    let base_url =
        std::env::var("LLM_BASE_URL").unwrap_or_else(|_| "https://openrouter.ai/api/v1".into());
    let api_key = std::env::var("LLM_API_KEY").unwrap_or_default();

    let form = reqwest::multipart::Form::new()
        .part(
            "file",
            reqwest::multipart::Part::bytes(bytes)
                .file_name(filename)
                .mime_str("audio/mpeg")?,
        )
        .text("model", "whisper-1");

    let resp = transcribe_client()
        .post(format!(
            "{}/audio/transcriptions",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("transcription HTTP {}", resp.status());
    }

    let json: serde_json::Value = resp.json().await?;
    json.get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned())
        .ok_or_else(|| anyhow::anyhow!("no 'text' field in transcription response"))
}

// ─── Utilities ───────────────────────────────────────────────────────────────

async fn download_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    let resp = download_client().get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("download HTTP {}", resp.status());
    }
    Ok(resp.bytes().await?.to_vec())
}

fn tempdir() -> anyhow::Result<PathBuf> {
    let p = std::env::temp_dir().join(format!("g10kz-media-{}", fastrand_u64()));
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

fn fastrand_u64() -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut h);
    h.finish()
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guess_mime_png() {
        assert_eq!(guess_mime(b"\x89PNG\r\n\x1a\n"), "image/png");
    }

    #[test]
    fn guess_mime_jpeg() {
        assert_eq!(guess_mime(b"\xff\xd8\xff\xe0"), "image/jpeg");
    }

    #[test]
    fn guess_mime_unknown_defaults_jpeg() {
        assert_eq!(guess_mime(b"randomdata"), "image/jpeg");
    }

    #[tokio::test]
    async fn process_image_https_passthrough() {
        let url = "https://example.com/image.png";
        let out = process_image(url).await.unwrap();
        assert_eq!(out.parts.len(), 1);
        match &out.parts[0] {
            Part::ImageUrl { url: u } => assert_eq!(u, url),
            _ => panic!("expected ImageUrl"),
        }
    }

    #[test]
    fn tempdir_creates_dir() {
        let p = tempdir().unwrap();
        assert!(p.exists());
        std::fs::remove_dir_all(&p).ok();
    }
}
