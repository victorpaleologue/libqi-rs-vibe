//! `ALAudioDevice`: the microphones and speakers.
//!
//! Clients receive audio by registering a service of their own that exposes
//! `processRemote(nbOfChannels, nbOfSamplesByChannel, timestamp, buffer)`, then calling
//! `setClientPreferences` and `subscribe` with the name of that service. The device then looks
//! the service up in the space and pushes buffers of 16-bit PCM to it periodically.

use super::Context;
use crate::naoqi_sim::{alvalue::AlValue, error, lock, now_secs_usecs};
use qi::{dynamic::ObjectBuilder, value::AsRaw, AnyObject, ObjectExt};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

/// The number of samples per channel of a 48 kHz buffer, like NAOqi.
pub const SAMPLES_PER_BUFFER_48K: u32 = 8192;

/// The number of consecutive failures to reach a subscriber before its subscription is dropped.
const MAX_FAILURES: u32 = 30;

/// The preferences of an audio client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientPreferences {
    /// The sample rate: 16000 or 48000 Hz.
    pub sample_rate: i32,
    /// The channel configuration: 0 for all four channels, 1 to 4 for a single channel.
    pub channels: i32,
    /// Whether the buffers are deinterleaved (one block per channel).
    pub deinterleaved: bool,
}

impl Default for ClientPreferences {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 0,
            deinterleaved: false,
        }
    }
}

impl ClientPreferences {
    /// The number of channels of the buffers.
    pub fn channel_count(&self) -> u32 {
        match self.channels {
            1..=4 => 1,
            _ => 4,
        }
    }

    /// The number of samples per channel of the buffers.
    pub fn samples_per_channel(&self) -> u32 {
        // 8192 samples at 48 kHz, the same duration at other rates.
        let rate = f64::from(self.sample_rate.max(1));
        let samples = f64::from(SAMPLES_PER_BUFFER_48K) * rate / 48_000.0;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            samples.floor() as u32
        }
    }

    /// The duration of a buffer.
    pub fn period(&self) -> Duration {
        let rate = self.sample_rate.max(1) as f64;
        Duration::from_secs_f64(f64::from(self.samples_per_channel()) / rate)
    }
}

/// The state of the audio device. Cheap to clone: clones share the state.
#[derive(Clone)]
pub struct AudioDevice(Arc<Inner>);

struct Inner {
    context: Context,
    preferences: Mutex<HashMap<String, ClientPreferences>>,
    subscriptions: Mutex<HashMap<String, tokio::task::AbortHandle>>,
    output_volume: AtomicI32,
    muted: AtomicBool,
    energy_computation: AtomicBool,
}

impl AudioDevice {
    /// Creates the audio device of a robot.
    pub fn new(context: &Context) -> Self {
        Self(Arc::new(Inner {
            context: context.clone(),
            preferences: Mutex::default(),
            subscriptions: Mutex::default(),
            output_volume: AtomicI32::new(50),
            muted: AtomicBool::new(false),
            energy_computation: AtomicBool::new(false),
        }))
    }

    /// The names of the current subscribers.
    pub fn subscribers(&self) -> Vec<String> {
        let mut names: Vec<String> = lock(&self.0.subscriptions).keys().cloned().collect();
        names.sort();
        names
    }

    /// The preferences of a client, or the defaults.
    pub fn preferences(&self, name: &str) -> ClientPreferences {
        lock(&self.0.preferences)
            .get(name)
            .copied()
            .unwrap_or_default()
    }

    /// Sets the preferences of a client.
    pub fn set_preferences(&self, name: &str, preferences: ClientPreferences) {
        lock(&self.0.preferences).insert(name.to_owned(), preferences);
    }

    /// Starts streaming audio to the service named `name`, which must be registered in the
    /// space (by the client itself, usually).
    pub fn subscribe(&self, name: &str) {
        let preferences = self.preferences(name);
        let device = self.clone();
        let subscriber = name.to_owned();
        let task = tokio::spawn(async move {
            device.stream(subscriber.clone(), preferences).await;
            lock(&device.0.subscriptions).remove(&subscriber);
        });
        if let Some(previous) =
            lock(&self.0.subscriptions).insert(name.to_owned(), task.abort_handle())
        {
            previous.abort();
        }
        self.0.context.logs.info(
            "ALAudioDevice",
            format!(
                "{name} subscribed ({} Hz, {} channels, {})",
                preferences.sample_rate,
                preferences.channel_count(),
                if preferences.deinterleaved {
                    "deinterleaved"
                } else {
                    "interleaved"
                }
            ),
        );
    }

    /// Stops streaming audio to a subscriber. Returns false if it was not subscribed.
    pub fn unsubscribe(&self, name: &str) -> bool {
        match lock(&self.0.subscriptions).remove(name) {
            Some(task) => {
                task.abort();
                self.0
                    .context
                    .logs
                    .info("ALAudioDevice", format!("{name} unsubscribed"));
                true
            }
            None => false,
        }
    }

    /// The streaming task of a subscriber: looks the subscriber service up in the space and
    /// calls its `processRemote` method with a buffer every period.
    async fn stream(&self, name: String, preferences: ClientPreferences) {
        let logs = &self.0.context.logs;
        let channels = preferences.channel_count();
        let samples = preferences.samples_per_channel();
        let mut interval = tokio::time::interval(preferences.period());
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut target: Option<AnyObject> = None;
        let mut failures = 0u32;
        let mut phase = 0.0f64;
        loop {
            interval.tick().await;
            let Ok(node) = self.0.context.node() else {
                break;
            };
            if target.is_none() {
                match node.service(&name).await {
                    Ok(service) => target = Some(service),
                    Err(err) => {
                        failures += 1;
                        if failures == 1 {
                            logs.warn(
                                "ALAudioDevice",
                                format!("cannot find subscriber service {name}: {err}"),
                            );
                        }
                        if failures >= MAX_FAILURES {
                            logs.warn("ALAudioDevice", format!("giving up on subscriber {name}"));
                            break;
                        }
                        continue;
                    }
                }
            }
            let Some(service) = &target else {
                continue;
            };
            let buffer = synthesize_pcm(
                channels,
                samples,
                preferences.sample_rate,
                preferences.deinterleaved,
                &mut phase,
            );
            let (secs, usecs) = now_secs_usecs();
            let timestamp = AlValue::list([AlValue::from(secs), AlValue::from(usecs)]);
            let result: qi::Result<()> = service
                .call(
                    "processRemote",
                    (
                        i32::try_from(channels).unwrap_or(4),
                        i32::try_from(samples).unwrap_or(0),
                        timestamp,
                        AsRaw(buffer),
                    ),
                )
                .await;
            match result {
                Ok(()) => failures = 0,
                Err(err) => {
                    failures += 1;
                    target = None;
                    logs.warn(
                        "ALAudioDevice",
                        format!("processRemote failed on subscriber {name}: {err}"),
                    );
                    if failures >= MAX_FAILURES {
                        logs.warn("ALAudioDevice", format!("giving up on subscriber {name}"));
                        break;
                    }
                }
            }
        }
    }

    /// Builds the `ALAudioDevice` service object.
    pub fn object(&self) -> AnyObject {
        let device = self.clone();
        let mut builder = ObjectBuilder::new();
        builder.set_description("The ALAudioDevice module allows other modules to access to the sound data of the nao's microphones, and to send sound toward its loudspeakers.");
        method!(builder, "setClientPreferences", [device], |(
            name,
            sample_rate,
            channels,
            deinterleaved,
        ): (
            String,
            i32,
            i32,
            i32
        )| {
            if sample_rate != 16_000 && sample_rate != 48_000 {
                return Err(error(format!(
                    "ALAudioDevice::setClientPreferences\n\tunsupported sample rate: {sample_rate}"
                )));
            }
            if !(0..=4).contains(&channels) {
                return Err(error(format!("ALAudioDevice::setClientPreferences\n\tinvalid channel configuration: {channels}")));
            }
            device.set_preferences(
                &name,
                ClientPreferences {
                    sample_rate,
                    channels,
                    deinterleaved: deinterleaved != 0,
                },
            );
            Ok(())
        });
        method!(builder, "subscribe", [device], |name: String| {
            device.subscribe(&name);
            Ok(())
        });
        method!(builder, "unsubscribe", [device], |name: String| {
            if device.unsubscribe(&name) {
                Ok(())
            } else {
                Err(error(format!(
                    "ALAudioDevice::unsubscribe\n\tunknown subscriber: {name}"
                )))
            }
        });
        method!(builder, "getSubscribers", [device], |(): ()| {
            Ok(device.subscribers())
        });
        method!(builder, "setOutputVolume", [device], |volume: i32| {
            device
                .0
                .output_volume
                .store(volume.clamp(0, 100), Ordering::SeqCst);
            device
                .0
                .context
                .memory
                .insert("ALAudioDevice/OutputVolume", volume.clamp(0, 100));
            Ok(())
        });
        method!(builder, "getOutputVolume", [device], |(): ()| {
            Ok(device.0.output_volume.load(Ordering::SeqCst))
        });
        method!(builder, "muteAudioOut", [device], |mute: bool| {
            device.0.muted.store(mute, Ordering::SeqCst);
            Ok(())
        });
        method!(builder, "isAudioOutMuted", [device], |(): ()| {
            Ok(device.0.muted.load(Ordering::SeqCst))
        });
        method!(builder, "enableEnergyComputation", [device], |(): ()| {
            device.0.energy_computation.store(true, Ordering::SeqCst);
            Ok(())
        });
        method!(builder, "disableEnergyComputation", [device], |(): ()| {
            device.0.energy_computation.store(false, Ordering::SeqCst);
            Ok(())
        });
        for (name, energy) in [
            ("getFrontMicEnergy", 1200.0f32),
            ("getRearMicEnergy", 900.0),
            ("getLeftMicEnergy", 1000.0),
            ("getRightMicEnergy", 1100.0),
        ] {
            method!(builder, name, [device], |(): ()| {
                if device.0.energy_computation.load(Ordering::SeqCst) {
                    Ok(energy)
                } else {
                    Err(error("ALAudioDevice\n\tenergy computation is disabled"))
                }
            });
        }
        method!(builder, "setParameter", [device], |(name, value): (
            String,
            i32
        )| {
            device
                .0
                .context
                .memory
                .insert(format!("ALAudioDevice/Parameter/{name}"), value);
            Ok(())
        });
        method!(builder, "getParameter", [device], |name: String| {
            Ok(device
                .0
                .context
                .memory
                .get_i32(&format!("ALAudioDevice/Parameter/{name}"))
                .unwrap_or(0))
        });
        method!(
            builder,
            "sendRemoteBufferToOutput",
            [device],
            |(samples, _buffer): (i32, AsRaw<Vec<u8>>)| {
                device.0.context.logs.verbose(
                    "ALAudioDevice",
                    format!("{samples} samples sent to the loudspeakers"),
                );
                Ok(true)
            }
        );
        method!(builder, "flushAudioOutputs", [], |(): ()| { Ok(()) });
        method!(
            builder,
            "startMicrophonesRecording",
            [device],
            |path: String| {
                device
                    .0
                    .context
                    .logs
                    .info("ALAudioDevice", format!("recording microphones to {path}"));
                Ok(())
            }
        );
        method!(builder, "stopMicrophonesRecording", [], |(): ()| { Ok(()) });
        AnyObject::new(builder.build())
    }
}

impl std::fmt::Debug for AudioDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioDevice")
            .field("subscribers", &self.subscribers())
            .finish_non_exhaustive()
    }
}

/// Synthesizes a buffer of 16-bit PCM: a sine of a different pitch on each channel. The phase
/// is carried from one buffer to the next.
pub fn synthesize_pcm(
    channels: u32,
    samples: u32,
    sample_rate: i32,
    deinterleaved: bool,
    phase: &mut f64,
) -> Vec<u8> {
    let rate = f64::from(sample_rate.max(1));
    let (channels_usize, samples_usize) = (channels as usize, samples as usize);
    let mut buffer = vec![0u8; channels_usize * samples_usize * 2];
    let start = *phase;
    for sample in 0..samples_usize {
        let t = start + sample as f64 / rate;
        for channel in 0..channels_usize {
            let frequency = 440.0 * (1.0 + channel as f64 * 0.25);
            let value = (t * frequency * std::f64::consts::TAU).sin() * 8000.0;
            let value = value as i16;
            let index = if deinterleaved {
                channel * samples_usize + sample
            } else {
                sample * channels_usize + channel
            };
            buffer[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
        }
    }
    *phase = start + samples_usize as f64 / rate;
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_follow_naoqi_defaults() {
        let preferences = ClientPreferences::default();
        assert_eq!(preferences.channel_count(), 4);
        assert_eq!(preferences.samples_per_channel(), 8192);
        assert!(
            preferences.period() > Duration::from_millis(170)
                && preferences.period() < Duration::from_millis(171)
        );
        let mono = ClientPreferences {
            sample_rate: 16_000,
            channels: 3,
            deinterleaved: true,
        };
        assert_eq!(mono.channel_count(), 1);
        assert_eq!(mono.samples_per_channel(), 2730);
    }

    #[test]
    fn pcm_buffers_have_the_right_size() {
        let mut phase = 0.0;
        let buffer = synthesize_pcm(4, 8192, 48_000, false, &mut phase);
        assert_eq!(buffer.len(), 2 * 4 * 8192);
        assert!(buffer.iter().any(|byte| *byte != 0));
        assert!((phase - 8192.0 / 48_000.0).abs() < 1e-9);
    }
}
