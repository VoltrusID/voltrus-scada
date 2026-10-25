//! Audible annunciation: a tone loops while any open alarm is
//! unacknowledged, its pattern set by the worst such alarm's priority, until
//! the operator acknowledges, shelves or mutes. The tones are synthesized
//! (no sound file, nothing to license): a fast two-tone warble for
//! critical, a double beep every 2.5 s for warning, a soft chime every 5 s
//! for the rest.
//!
//! The pattern repeats on the audio device's own thread; the console only
//! says which tone should sound when the alarm list changes, so the
//! annunciator adds no timer and no frame. The mute choice is remembered
//! per user and server in `<config dir>/voltrus-native/alarm-sound.toml`.

use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use serde::{Deserialize, Serialize};
use voltrus_scada_native::api::Alarm;

use super::alarms::{is_resolved, priority};
use super::{is_shelved, severity_rank};

/// The pattern of the worst unacknowledged alarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tone {
    Critical,
    Warning,
    Info,
}

impl Tone {
    /// What should sound for these alarms: the worst priority among the
    /// open ones nobody acknowledged; `None` when there are none.
    pub fn worst(alarms: &[Alarm]) -> Option<Self> {
        alarms
            .iter()
            .filter(|alarm| {
                alarm.acknowledged_at.is_none() && !is_shelved(alarm) && !is_resolved(alarm)
            })
            .map(|alarm| match severity_rank(priority(alarm)) {
                0 => Self::Critical,
                1 | 2 => Self::Warning,
                _ => Self::Info,
            })
            .min()
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }

    /// The beeps and gaps: (frequency in Hz, 0 for silence; seconds).
    pub fn steps(self) -> &'static [(f32, f32)] {
        match self {
            Self::Critical => &[
                (988.0, 0.14),
                (0.0, 0.04),
                (784.0, 0.14),
                (0.0, 0.04),
                (988.0, 0.14),
                (0.0, 0.04),
                (784.0, 0.14),
                (0.0, 0.66),
            ],
            Self::Warning => &[(660.0, 0.22), (0.0, 0.12), (660.0, 0.22), (0.0, 1.94)],
            Self::Info => &[(523.25, 0.35), (0.0, 4.65)],
        }
    }

    /// One loop of the pattern, in seconds.
    pub fn period(self) -> f32 {
        self.steps().iter().map(|(_, length)| length).sum()
    }

    fn code(tone: Option<Self>) -> u8 {
        match tone {
            None => 0,
            Some(Self::Critical) => 1,
            Some(Self::Warning) => 2,
            Some(Self::Info) => 3,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Critical),
            2 => Some(Self::Warning),
            3 => Some(Self::Info),
            _ => None,
        }
    }
}

/// Loudness of a beep's peak, of full scale.
const LEVEL: f32 = 0.22;
/// Each beep fades in and out over this long, so it does not click.
const RAMP: f32 = 0.008;

/// The tone's sample `at` seconds into its loop.
pub fn sample(tone: Tone, at: f32) -> f32 {
    let mut at = at.rem_euclid(tone.period());
    for &(frequency, length) in tone.steps() {
        if at < length {
            if frequency <= 0.0 {
                return 0.0;
            }
            let envelope = (at / RAMP).min((length - at) / RAMP).clamp(0.0, 1.0);
            // A touch of the octave makes the beep carry over plant noise.
            let wave = (TAU * frequency * at).sin() + 0.25 * (TAU * 2.0 * frequency * at).sin();
            return LEVEL * envelope * wave / 1.25;
        }
        at -= length;
    }
    0.0
}

/// The open output stream and the tone its callback plays.
struct Output {
    stream: cpal::Stream,
    tone: Arc<AtomicU8>,
    playing: bool,
}

impl Output {
    fn open() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no sound output device")?;
        let supported = device
            .default_output_config()
            .map_err(|error| error.to_string())?;
        let config = supported.config();
        let tone = Arc::new(AtomicU8::new(0));
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, &config, tone.clone()),
            SampleFormat::I16 => build::<i16>(&device, &config, tone.clone()),
            SampleFormat::U16 => build::<u16>(&device, &config, tone.clone()),
            SampleFormat::I32 => build::<i32>(&device, &config, tone.clone()),
            other => return Err(format!("unsupported sample format {other}")),
        }
        .map_err(|error| error.to_string())?;
        Ok(Self {
            stream,
            tone,
            playing: false,
        })
    }

    fn set(&mut self, tone: Option<Tone>) -> Result<(), String> {
        self.tone.store(Tone::code(tone), Ordering::Relaxed);
        match (tone.is_some(), self.playing) {
            (true, false) => self.stream.play().map_err(|error| error.to_string())?,
            // Paused while silent: the device thread sleeps.
            (false, true) => self.stream.pause().map_err(|error| error.to_string())?,
            _ => {}
        }
        self.playing = tone.is_some();
        Ok(())
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    tone: Arc<AtomicU8>,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = usize::from(config.channels).max(1);
    #[allow(clippy::cast_precision_loss)]
    let rate = config.sample_rate as f32;
    let mut playing: Option<Tone> = None;
    let mut frame: u64 = 0;
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let now = Tone::from_code(tone.load(Ordering::Relaxed));
            if now != playing {
                // A new pattern starts from its first beep.
                playing = now;
                frame = 0;
            }
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss
            )]
            let period = playing.map_or(1, |tone| ((tone.period() * rate) as u64).max(1));
            for samples in data.chunks_mut(channels) {
                #[allow(clippy::cast_precision_loss)]
                let value = playing.map_or(0.0, |tone| sample(tone, frame as f32 / rate));
                frame = (frame + 1) % period;
                for out in samples {
                    *out = T::from_sample(value);
                }
            }
        },
        |error| eprintln!("voltrus-native: alarm sound: {error}"),
        None,
    )
}

/// The annunciator, kept as console state (`console.state::<Annunciator>()`).
#[derive(Default)]
pub struct Annunciator {
    /// Whose mute choice is loaded: `user@server`.
    user: Option<String>,
    muted: bool,
    /// What the alarms call for, muted or not.
    wanted: Option<Tone>,
    output: Option<Output>,
    /// Why there is no sound, once opening the device failed.
    failed: Option<String>,
}

impl Annunciator {
    /// Make the sound follow `alarms` for `user` (`None` when signed out).
    pub fn sync(&mut self, user: Option<String>, alarms: &[Alarm]) {
        if user != self.user {
            self.muted = user.as_deref().is_some_and(|user| load().is_muted(user));
            self.user = user;
            // A new session may find the device it lacked.
            self.failed = None;
        }
        self.wanted = if self.user.is_some() {
            Tone::worst(alarms)
        } else {
            None
        };
        self.apply();
    }

    /// Mute or unmute for the signed-in user, and remember it.
    pub fn toggle_mute(&mut self) -> bool {
        self.muted = !self.muted;
        if let Some(user) = &self.user {
            let mut prefs = load();
            prefs.set_muted(user, self.muted);
            save(&prefs);
        }
        self.apply();
        self.muted
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    /// The tone the alarms call for, muted or not.
    pub fn wanted(&self) -> Option<Tone> {
        self.wanted
    }

    /// What plays now: nothing while muted.
    pub fn sounding(&self) -> Option<Tone> {
        audible(self.wanted, self.muted)
    }

    /// Why nothing can sound, when the device could not be opened.
    pub fn unavailable(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    fn apply(&mut self) {
        let tone = self.sounding();
        if tone.is_some() && self.output.is_none() && self.failed.is_none() {
            match Output::open() {
                Ok(output) => self.output = Some(output),
                Err(error) => {
                    eprintln!("voltrus-native: no alarm sound: {error}");
                    self.failed = Some(error);
                }
            }
        }
        if let Some(output) = &mut self.output
            && let Err(error) = output.set(tone)
        {
            eprintln!("voltrus-native: alarm sound: {error}");
        }
    }
}

/// What should play: the alarms' tone unless muted.
pub fn audible(wanted: Option<Tone>, muted: bool) -> Option<Tone> {
    wanted.filter(|_| !muted)
}

/// The remembered mute choices.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    /// `user@server` of everyone who muted the alarm sound.
    #[serde(default)]
    pub muted: Vec<String>,
}

impl Prefs {
    pub fn is_muted(&self, user: &str) -> bool {
        self.muted.iter().any(|muted| muted == user)
    }

    pub fn set_muted(&mut self, user: &str, muted: bool) {
        self.muted.retain(|entry| entry != user);
        if muted {
            self.muted.push(user.to_owned());
        }
    }
}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("voltrus-native").join("alarm-sound.toml"))
}

fn load() -> Prefs {
    path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// Failures cost only the memory of the choice: reported, then ignored.
fn save(prefs: &Prefs) {
    let Some(path) = path() else {
        return;
    };
    let result = toml::to_string_pretty(prefs)
        .map_err(std::io::Error::other)
        .and_then(|text| {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let partial = path.with_extension("toml.partial");
            std::fs::write(&partial, text)?;
            std::fs::rename(&partial, &path)
        });
    if let Err(error) = result {
        eprintln!(
            "voltrus-native: could not remember the alarm sound choice in {}: {error}",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alarm(priority: &str) -> Alarm {
        Alarm {
            priority: priority.into(),
            severity: priority.into(),
            state: "active".into(),
            ..Alarm::default()
        }
    }

    #[test]
    fn the_worst_unacknowledged_open_alarm_picks_the_tone() {
        assert_eq!(Tone::worst(&[]), None);
        assert_eq!(
            Tone::worst(&[alarm("info"), alarm("warning")]),
            Some(Tone::Warning)
        );
        let acknowledged = Alarm {
            acknowledged_at: Some(1),
            ..alarm("critical")
        };
        let shelved = Alarm {
            state: "shelved".into(),
            ..alarm("critical")
        };
        let resolved = Alarm {
            resolved_at: Some(2),
            ..alarm("critical")
        };
        assert_eq!(
            Tone::worst(&[acknowledged, shelved, resolved.clone(), alarm("info")]),
            Some(Tone::Info),
            "acknowledged, shelved and resolved alarms are quiet"
        );
        assert_eq!(Tone::worst(&[resolved]), None);
        assert_eq!(
            Tone::worst(&[alarm("warning"), alarm("critical")]),
            Some(Tone::Critical)
        );
    }

    #[test]
    fn muting_silences_without_forgetting_the_tone() {
        assert_eq!(audible(Some(Tone::Critical), false), Some(Tone::Critical));
        assert_eq!(audible(Some(Tone::Critical), true), None);
        assert_eq!(audible(None, false), None);
        let mut annunciator = Annunciator::default();
        // Signed out: nothing is wanted, so no device is opened.
        annunciator.sync(None, &[alarm("critical")]);
        assert_eq!(annunciator.wanted(), None);
        assert_eq!(annunciator.sounding(), None);
    }

    #[test]
    fn patterns_loop_with_silent_gaps_and_soft_edges() {
        for tone in [Tone::Critical, Tone::Warning, Tone::Info] {
            let period = tone.period();
            assert!(
                period > 1.0 && period < 5.5,
                "{tone:?} loops every {period} s"
            );
            // The last step is a gap: the loop ends quiet.
            assert!(sample(tone, period - 0.01).abs() < 1e-9);
            // Each beep starts at zero (no click) and stays within range.
            assert!(sample(tone, 0.0).abs() < 1e-6);
            let peak = (0..2000)
                .map(|i| sample(tone, i as f32 * 0.0005).abs())
                .fold(0.0_f32, f32::max);
            assert!(peak > 0.05 && peak <= LEVEL, "{tone:?} peaks at {peak}");
            // It repeats.
            assert!((sample(tone, 0.1) - sample(tone, 0.1 + period)).abs() < 1e-3);
        }
        assert!(Tone::Critical.period() < Tone::Warning.period());
        assert!(Tone::Warning.period() < Tone::Info.period());
        for code in 0..4 {
            assert_eq!(Tone::code(Tone::from_code(code)), code);
        }
    }

    #[test]
    fn mute_choices_are_per_user_and_server() {
        let mut prefs = Prefs::default();
        prefs.set_muted("admin@http://plant:3000", true);
        prefs.set_muted("admin@http://plant:3000", true);
        prefs.set_muted("operator@http://plant:3000", true);
        prefs.set_muted("operator@http://plant:3000", false);
        assert_eq!(prefs.muted, ["admin@http://plant:3000"]);
        assert!(prefs.is_muted("admin@http://plant:3000"));
        assert!(!prefs.is_muted("admin@http://other:3000"));
        let text = toml::to_string_pretty(&prefs).unwrap();
        assert_eq!(toml::from_str::<Prefs>(&text).unwrap(), prefs);
        assert_eq!(toml::from_str::<Prefs>("").unwrap(), Prefs::default());
    }
}
