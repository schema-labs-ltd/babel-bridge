use super::*;

pub(super) async fn convert(
    dir: &FsPath,
    input: &str,
    output: &str,
    options: &[&str],
) -> Result<Vec<u8>> {
    convert_audio(dir, input, output, options, true).await
}
pub(super) async fn convert_audio(
    dir: &FsPath,
    input: &str,
    output: &str,
    options: &[&str],
    limit_duration: bool,
) -> Result<Vec<u8>> {
    let mut command = Command::new("ffmpeg");
    command
        .kill_on_drop(true)
        .args([
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(dir.join(input))
        .args([
            "-vn",
            "-map_metadata",
            "-1",
            "-threads",
            "1",
            "-fs",
            "16000000",
        ]);
    if limit_duration {
        command.args(["-t", "241"]);
    }
    command.args(options).arg(dir.join(output));
    let output_result = tokio::time::timeout(Duration::from_secs(45), command.output())
        .await
        .context("Audio conversion timed out")??;
    if !output_result.status.success() {
        bail!("Could not decode this audio. Please retry downloading the voice note.");
    }
    let bytes = tokio::fs::read(dir.join(output)).await?;
    if bytes.is_empty() || bytes.len() >= 16_000_000 {
        bail!("Recording is empty or too large.");
    }
    Ok(bytes)
}
/// Playback conversion is independent of AI translation and preserves the full recording.
pub async fn playable_audio(encoded: &str) -> Result<String> {
    static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    let _permit = tokio::time::timeout(Duration::from_secs(90), SLOTS.acquire())
        .await
        .context("Audio playback is busy. Please try again.")??;
    let bytes = decode_audio(encoded)?;
    let dir = Scratch::new(&std::env::temp_dir())?;
    tokio::fs::write(dir.0.join("source"), bytes).await?;
    let mp3 = convert_audio(
        &dir.0,
        "source",
        "playback.mp3",
        &["-codec:a", "libmp3lame", "-b:a", "96k"],
        false,
    )
    .await?;
    Ok(B64.encode(mp3))
}

pub(super) fn decode_audio(encoded: &str) -> Result<Vec<u8>> {
    if encoded.len() > MAX_BYTES.div_ceil(3) * 4 {
        bail!("Voice notes must be smaller than 16 MB.");
    }
    let bytes = B64.decode(encoded).context("Invalid audio encoding")?;
    if bytes.len() < 32 || bytes.len() > MAX_BYTES {
        bail!("Recording is empty or too large.");
    }
    Ok(bytes)
}

/// Acoustic pitch matching only: this does not identify a speaker's gender.
pub(super) fn select_voice(preference: &str, pcm: &[u8]) -> &'static str {
    match preference {
        "masculine" => return "onyx",
        "feminine" => return "shimmer",
        "neutral" => return "alloy",
        _ => {}
    }
    let samples: Vec<f64> = pcm
        .chunks_exact(2)
        .map(|x| i16::from_le_bytes([x[0], x[1]]) as f64 / 32768.0)
        .collect();
    let mut pitches = Vec::new();
    for frame in samples.chunks(640).step_by(5).take(200) {
        if frame.len() < 640 {
            continue;
        }
        let mean = frame.iter().sum::<f64>() / frame.len() as f64;
        let centered: Vec<_> = frame.iter().map(|s| s - mean).collect();
        let energy: f64 = centered.iter().map(|s| s * s).sum();
        if energy / (frame.len() as f64) < 0.0001 {
            continue;
        }
        let mut best = (0.0, 0);
        for lag in 40..=228 {
            // 70–400 Hz
            let a = &centered[..centered.len() - lag];
            let b = &centered[lag..];
            let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            let norm = (a.iter().map(|x| x * x).sum::<f64>()
                * b.iter().map(|x| x * x).sum::<f64>())
            .sqrt();
            let correlation = dot / norm.max(1e-12);
            if correlation > best.0 {
                best = (correlation, lag);
            }
            // First strong peak avoids selecting a lower octave of the same pitch.
            if best.0 > 0.9 && correlation < best.0 - 0.02 {
                break;
            }
        }
        if best.0 > 0.75 && best.1 > 0 {
            pitches.push(RATE as f64 / best.1 as f64);
        }
    }
    if pitches.len() < 4 {
        return "alloy";
    }
    pitches.sort_by(f64::total_cmp);
    let median = pitches[pitches.len() / 2];
    if median < 165.0 {
        "onyx"
    } else if median > 195.0 {
        "shimmer"
    } else {
        "alloy"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pcm_tone(hz: f64) -> Vec<u8> {
        (0..RATE)
            .flat_map(|i| {
                (((2.0 * std::f64::consts::PI * hz * i as f64 / RATE as f64).sin() * 15000.0)
                    as i16)
                    .to_le_bytes()
            })
            .collect()
    }
    #[test]
    fn acoustic_matching_and_overrides() {
        assert_eq!(select_voice("auto", &pcm_tone(120.0)), "onyx");
        assert_eq!(select_voice("auto", &pcm_tone(240.0)), "shimmer");
        assert_eq!(select_voice("auto", &vec![0; RATE * 2]), "alloy");
        assert_eq!(select_voice("feminine", &pcm_tone(120.0)), "shimmer");
        assert_eq!(select_voice("neutral", &pcm_tone(240.0)), "alloy");
    }
    #[test]
    fn invalid_audio_is_rejected() {
        assert!(decode_audio("bad").is_err());
        assert!(decode_audio("").is_err());
        assert!(!valid_voice("arbitrary-model-voice"));
    }
}
