//! The DCM plugin of the example: `RustEyes`, and the `RustDcm` service that reports on it.
//!
//! In the loop of the DCM, the plugin reads the position of the head yaw joint right after the
//! DCM updated it, and, while the animation is on, turns the blue LEDs of the eyes one segment
//! further every 100 ms. The service reads what the plugin shares.

use qi::module::naoqi::{Alias, Cycle, DcmPlugin, MemoryF32, Naoqi, Update};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

/// The state the plugin shares with the service, without locks: the plugin runs in the loop of
/// the DCM, which it must not delay.
pub struct DcmState {
    cycles: AtomicU64,
    head_yaw: AtomicU32,
    period: AtomicI32,
    led_animation: AtomicBool,
}

pub static DCM_STATE: DcmState = DcmState {
    cycles: AtomicU64::new(0),
    head_yaw: AtomicU32::new(0),
    period: AtomicI32::new(0),
    led_animation: AtomicBool::new(true),
};

/// The ALMemory key of the position of the head yaw joint, on NAO and Pepper.
pub const HEAD_YAW_KEY: &str = "Device/SubDeviceList/HeadYaw/Position/Sensor/Value";

/// The ALMemory keys of the blue LEDs of the eyes, left then right, by angle.
pub fn eye_led_keys() -> Vec<String> {
    ["Left", "Right"]
        .iter()
        .flat_map(|side| {
            (0..8).map(move |segment| {
                format!(
                    "Device/SubDeviceList/Face/Led/Blue/{side}/{}Deg/Actuator/Value",
                    segment * 45
                )
            })
        })
        .collect()
}

/// The number of cycles of the DCM between two steps of the animation (100 ms on NAO).
const CYCLES_PER_STEP: u64 = 10;

/// The DCM plugin.
pub struct RustEyes {
    head_yaw: MemoryF32,
    eyes: Option<Alias>,
    last_time: Option<i32>,
}

impl RustEyes {
    /// Creates the plugin, in the constructor of its ALModule.
    pub fn new(naoqi: &Naoqi<'_>) -> qi::Result<Self> {
        let head_yaw = naoqi.memory_f32(HEAD_YAW_KEY)?;
        let keys = eye_led_keys();
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        // A robot without these LEDs still runs the rest of the plugin.
        let eyes = match naoqi.create_alias("RustEyes", &keys) {
            Ok(alias) => Some(alias),
            Err(err) => {
                eprintln!("RustEyes: no eye LEDs, no animation: {err}");
                None
            }
        };
        Ok(Self {
            head_yaw,
            eyes,
            last_time: None,
        })
    }
}

impl DcmPlugin for RustEyes {
    fn pre_process(&mut self, cycle: &mut Cycle<'_>) {
        let cycles = DCM_STATE.cycles.fetch_add(1, Ordering::Relaxed) + 1;
        let Ok(time) = cycle.time(0) else {
            return;
        };
        if let Some(last) = self.last_time.replace(time) {
            DCM_STATE.period.store(time - last, Ordering::Relaxed);
        }
        let Some(eyes) = &self.eyes else {
            return;
        };
        if !DCM_STATE.led_animation.load(Ordering::Relaxed)
            || !cycles.is_multiple_of(CYCLES_PER_STEP)
        {
            return;
        }
        let step = (cycles / CYCLES_PER_STEP % 8) as usize;
        let values: Vec<f32> = (0..eyes.len())
            .map(|index| if index % 8 == step { 1.0 } else { 0.0 })
            .collect();
        if let Err(err) = cycle.set_alias(eyes, Update::ClearAll, time, &values) {
            eprintln!("RustEyes: {err}");
        }
    }

    fn post_process(&mut self, _cycle: &mut Cycle<'_>) {
        DCM_STATE
            .head_yaw
            .store(self.head_yaw.get().to_bits(), Ordering::Relaxed);
    }
}

/// The service reporting on the DCM plugin.
#[qi::object(case = "camelCase")]
pub trait DcmReport {
    /// The number of cycles of the DCM the plugin ran in.
    async fn cycles(&self) -> qi::Result<i64>;
    /// The position of the head yaw joint at the last cycle, in radians.
    async fn head_yaw(&self) -> qi::Result<f32>;
    /// The duration of the last cycle of the DCM, in milliseconds.
    async fn period(&self) -> qi::Result<i32>;
    /// Turns the animation of the eyes on or off.
    async fn set_led_animation(&self, on: bool) -> qi::Result<()>;
}

/// The implementation of [`DcmReport`].
pub struct RustDcm;

#[qi::async_trait]
impl DcmReport for RustDcm {
    async fn cycles(&self) -> qi::Result<i64> {
        Ok(DCM_STATE.cycles.load(Ordering::Relaxed) as i64)
    }

    async fn head_yaw(&self) -> qi::Result<f32> {
        Ok(f32::from_bits(DCM_STATE.head_yaw.load(Ordering::Relaxed)))
    }

    async fn period(&self) -> qi::Result<i32> {
        Ok(DCM_STATE.period.load(Ordering::Relaxed))
    }

    async fn set_led_animation(&self, on: bool) -> qi::Result<()> {
        DCM_STATE.led_animation.store(on, Ordering::Relaxed);
        Ok(())
    }
}
