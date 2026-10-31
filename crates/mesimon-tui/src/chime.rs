//! Mesimon's own two sounds (T-1094): `Chime` for an agent that needs you,
//! `Tick` for a finished turn.
//!
//! Synthesized here rather than shipped as recordings: no audio file in the
//! repository, no licence to carry, and the same sound from `afplay`,
//! `paplay` and Windows' `SoundPlayer`, which all read a plain 16-bit PCM
//! WAV. The bytes are made once per process and written beside the
//! notification icons by the same digest-named, atomic road
//! ([`crate::mascot::publish_asset`]), so a player is handed a path.
//!
//! The two differ in every way a person can tell without looking: the chime
//! is two bell notes rising a fifth over most of a second; the tick is one
//! short, quieter note that is gone in a quarter of one.

use std::f64::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use mesimon_core::notify::Sound;

const RATE: u32 = 44_100;

/// One partial of a struck note: its frequency as a multiple of the note's,
/// its level, and how fast it dies away (the time constant, in seconds).
/// Higher partials die faster, which is what makes a strike sound struck.
type Partial = (f64, f64, f64);

/// A small bell: the fundamental, a quieter octave, a faint twelfth, and a
/// slightly detuned double of the fundamental whose slow beating is the
/// shimmer a struck glass has.
const BELL: &[Partial] =
    &[(1.0, 1.0, 0.34), (1.0035, 0.18, 0.34), (2.0, 0.26, 0.14), (3.0, 0.07, 0.07)];

/// A soft mallet on wood: the fundamental and the fourth harmonic a marimba
/// bar is tuned to, both short.
const MALLET: &[Partial] = &[(1.0, 1.0, 0.055), (4.0, 0.16, 0.014)];

/// The file for one of mesimon's own sounds, written if it is not there.
/// An error for any other sound: those are the platform's files.
pub(crate) fn file(dir: &Path, sound: Sound) -> std::io::Result<PathBuf> {
    let bytes = wav(sound).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "not one of mesimon's sounds")
    })?;
    crate::mascot::publish_asset(dir, &sound.key().to_ascii_lowercase(), "wav", bytes)
}

/// The WAV bytes of one of mesimon's own sounds, made once per process.
pub(crate) fn wav(sound: Sound) -> Option<&'static [u8]> {
    static CHIME: OnceLock<Vec<u8>> = OnceLock::new();
    static TICK: OnceLock<Vec<u8>> = OnceLock::new();
    match sound {
        Sound::Chime => Some(CHIME.get_or_init(chime)),
        Sound::Tick => Some(TICK.get_or_init(tick)),
        _ => None,
    }
}

/// E5 then B5, 140 ms apart: a rising fifth, the interval of a call.
fn chime() -> Vec<u8> {
    let mut out = vec![0.0; samples(0.95)];
    strike(&mut out, 0.0, 659.26, 1.0, 0.004, BELL);
    strike(&mut out, 0.14, 987.77, 0.85, 0.004, BELL);
    encode(&out, 0.55)
}

/// C6, once, and quieter than the chime: the finished turn is news, not an
/// alarm.
fn tick() -> Vec<u8> {
    let mut out = vec![0.0; samples(0.26)];
    strike(&mut out, 0.0, 1046.5, 1.0, 0.002, MALLET);
    encode(&out, 0.38)
}

fn samples(seconds: f64) -> usize {
    (seconds * f64::from(RATE)).round() as usize
}

/// Add one struck note to `out`, from `start` seconds: a linear attack of
/// `attack` seconds, then each partial's own exponential decay.
fn strike(out: &mut [f64], start: f64, freq: f64, gain: f64, attack: f64, partials: &[Partial]) {
    let first = samples(start);
    for (i, sample) in out.iter_mut().enumerate().skip(first) {
        let t = (i - first) as f64 / f64::from(RATE);
        let rise = (t / attack).min(1.0);
        let voice: f64 = partials
            .iter()
            .map(|&(ratio, level, decay)| {
                level * (-t / decay).exp() * (TAU * freq * ratio * t).sin()
            })
            .sum();
        *sample += gain * rise * voice;
    }
}

/// Normalize to `peak` (of full scale), fade the last 20 ms to silence so
/// the end cannot click, and wrap as mono 16-bit PCM.
fn encode(out: &[f64], peak: f64) -> Vec<u8> {
    let loudest = out.iter().fold(0.0_f64, |m, s| m.max(s.abs())).max(f64::EPSILON);
    let fade = samples(0.02).min(out.len());
    let pcm: Vec<i16> = out
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let left = out.len() - 1 - i;
            let tail = if left < fade { left as f64 / fade as f64 } else { 1.0 };
            (s / loudest * peak * tail * f64::from(i16::MAX)).round() as i16
        })
        .collect();
    let data = (pcm.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes per second
    b.extend_from_slice(&2u16.to_le_bytes()); // block align
    b.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(bytes: &[u8]) -> Vec<i16> {
        bytes[44..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect()
    }

    #[test]
    fn both_sounds_are_plain_pcm_wavs_any_player_reads() {
        for sound in [Sound::Chime, Sound::Tick] {
            let b = wav(sound).expect("an own sound");
            assert_eq!(&b[..4], b"RIFF");
            assert_eq!(&b[8..16], b"WAVEfmt ");
            assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize, b.len() - 8);
            assert_eq!(u16::from_le_bytes([b[20], b[21]]), 1, "uncompressed PCM");
            assert_eq!(u16::from_le_bytes([b[22], b[23]]), 1, "mono");
            assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), RATE);
            assert_eq!(&b[36..40], b"data");
            assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()) as usize, b.len() - 44);
        }
        assert!(wav(Sound::Glass).is_none() && wav(Sound::Off).is_none());
    }

    #[test]
    fn the_two_are_told_apart_by_length_and_loudness_and_neither_clicks() {
        let chime = pcm(wav(Sound::Chime).unwrap());
        let tick = pcm(wav(Sound::Tick).unwrap());
        assert!(chime.len() > 3 * tick.len(), "the chime rings on; the tick is gone");
        let peak = |s: &[i16]| s.iter().map(|v| v.unsigned_abs()).max().unwrap();
        assert!(peak(&tick) < peak(&chime), "the finished turn is the quieter");
        assert!(f64::from(peak(&chime)) < 0.6 * f64::from(i16::MAX), "headroom, never clipping");
        for s in [&chime, &tick] {
            assert_eq!(*s.last().unwrap(), 0, "faded to silence");
            assert!(s[0].unsigned_abs() < 100, "starts from silence");
        }
    }

    #[test]
    fn written_once_under_a_digest_name() {
        let dir = std::env::temp_dir().join(format!("msmn-chime-{}", uuid::Uuid::new_v4()));
        let chime = file(&dir, Sound::Chime).unwrap();
        let tick = file(&dir, Sound::Tick).unwrap();
        assert_ne!(chime, tick);
        assert!(chime.file_name().unwrap().to_string_lossy().starts_with("chime-"));
        assert!(chime.extension().is_some_and(|e| e == "wav"));
        assert_eq!(std::fs::read(&chime).unwrap(), wav(Sound::Chime).unwrap());
        assert_eq!(file(&dir, Sound::Chime).unwrap(), chime, "the same name next time");
        assert!(file(&dir, Sound::Glass).is_err(), "the platform's sounds are not ours to write");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
