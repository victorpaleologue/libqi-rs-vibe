//! `ALLeds`: the LEDs of the robot.
//!
//! The color and intensity of each group are kept in the memory as the `ALLeds/<group>/Color`
//! (an `0x00RRGGBB` integer) and `ALLeds/<group>/Intensity` keys.

use super::Context;
use crate::naoqi_sim::{error, lock};
use qi::{call, dynamic::ObjectBuilder, AnyObject, Error};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

/// The state of a group of LEDs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LedState {
    /// The color as `0x00RRGGBB`.
    pub color: i32,
    /// The intensity, between 0 and 1.
    pub intensity: f32,
}

/// The state of the LEDs. Cheap to clone: clones share the state.
#[derive(Clone)]
pub struct Leds(Arc<Inner>);

struct Inner {
    context: Context,
    groups: Mutex<HashMap<String, LedState>>,
    custom_groups: Mutex<HashMap<String, Vec<String>>>,
}

impl Leds {
    /// Creates the LEDs of a robot, all white and on.
    pub fn new(context: &Context) -> Self {
        let groups = context
            .robot
            .description()
            .led_groups
            .iter()
            .map(|group| {
                (
                    (*group).to_owned(),
                    LedState {
                        color: 0x00FF_FFFF,
                        intensity: 1.0,
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        let leds = Self(Arc::new(Inner {
            context: context.clone(),
            groups: Mutex::new(groups),
            custom_groups: Mutex::default(),
        }));
        for group in context.robot.description().led_groups {
            leds.publish(group);
        }
        leds
    }

    /// The groups of LEDs, predefined and created.
    pub fn groups(&self) -> Vec<String> {
        let mut groups: Vec<String> = lock(&self.0.groups).keys().cloned().collect();
        groups.sort();
        groups
    }

    /// The state of a group.
    pub fn state(&self, group: &str) -> Option<LedState> {
        lock(&self.0.groups).get(group).copied()
    }

    fn update(&self, group: &str, update: impl Fn(&mut LedState)) -> qi::Result<()> {
        {
            let mut groups = lock(&self.0.groups);
            let state = groups
                .get_mut(group)
                .ok_or_else(|| error(format!("ALLeds\n\tunknown LED or group: {group}")))?;
            update(state);
        }
        self.publish(group);
        Ok(())
    }

    fn publish(&self, group: &str) {
        if let Some(state) = self.state(group) {
            let memory = &self.0.context.memory;
            memory.insert(format!("ALLeds/{group}/Color"), state.color);
            memory.insert(format!("ALLeds/{group}/Intensity"), state.intensity);
        }
    }

    /// Builds the `ALLeds` service object.
    pub fn object(&self) -> AnyObject {
        let leds = self.clone();
        let mut builder = ObjectBuilder::new();
        builder.set_description(
            "ALLeds gives access to the LEDs of the robot: colors, intensities and groups.",
        );
        method!(builder, "fadeRGB", [leds], |(group, color, duration): (
            String,
            i32,
            f32
        )| {
            leds.0.context.logs.verbose(
                "ALLeds",
                format!("fadeRGB {group} to {color:#08x} in {duration}s"),
            );
            wait(duration).await?;
            leds.update(&group, |state| {
                state.color = color & 0x00FF_FFFF;
                state.intensity = 1.0;
            })
        });
        method!(
            builder,
            "fadeListRGB",
            [leds],
            |(group, colors, times): (String, Vec<i32>, Vec<f32>)| {
                for (color, time) in colors
                    .iter()
                    .zip(times.iter().chain(std::iter::repeat(&0.0)))
                {
                    wait(*time).await?;
                    leds.update(&group, |state| {
                        state.color = color & 0x00FF_FFFF;
                        state.intensity = 1.0;
                    })?;
                }
                Ok(())
            }
        );
        method!(builder, "fade", [leds], |(group, intensity, duration): (
            String,
            f32,
            f32
        )| {
            wait(duration).await?;
            leds.update(&group, |state| state.intensity = intensity.clamp(0.0, 1.0))
        });
        method!(builder, "setIntensity", [leds], |(group, intensity): (
            String,
            f32
        )| {
            leds.update(&group, |state| state.intensity = intensity.clamp(0.0, 1.0))
        });
        method!(builder, "getIntensity", [leds], |group: String| {
            leds.state(&group)
                .map(|state| state.intensity)
                .ok_or_else(|| {
                    error(format!(
                        "ALLeds::getIntensity\n\tunknown LED or group: {group}"
                    ))
                })
        });
        method!(builder, "on", [leds], |group: String| {
            leds.update(&group, |state| state.intensity = 1.0)
        });
        method!(builder, "off", [leds], |group: String| {
            leds.update(&group, |state| state.intensity = 0.0)
        });
        method!(builder, "reset", [leds], |group: String| {
            leds.update(&group, |state| {
                state.color = 0x00FF_FFFF;
                state.intensity = 1.0;
            })
        });
        method!(builder, "listGroups", [leds], |(): ()| {
            Ok(leds.groups())
        });
        method!(builder, "listLEDs", [leds], |(): ()| { Ok(leds.groups()) });
        method!(builder, "listGroup", [leds], |group: String| {
            if let Some(members) = lock(&leds.0.custom_groups).get(&group) {
                return Ok(members.clone());
            }
            leds.state(&group)
                .map(|_| vec![group.clone()])
                .ok_or_else(|| error(format!("ALLeds::listGroup\n\tunknown group: {group}")))
        });
        method!(builder, "createGroup", [leds], |(group, members): (
            String,
            Vec<String>
        )| {
            lock(&leds.0.custom_groups).insert(group.clone(), members);
            lock(&leds.0.groups).insert(
                group.clone(),
                LedState {
                    color: 0x00FF_FFFF,
                    intensity: 1.0,
                },
            );
            leds.publish(&group);
            Ok(())
        });
        method!(builder, "randomEyes", [leds], |duration: f32| {
            wait(duration).await?;
            leds.update("FaceLeds", |state| state.color = 0x0000_FF80)
        });
        method!(builder, "rasta", [leds], |duration: f32| {
            wait(duration).await?;
            leds.update("FaceLeds", |state| state.color = 0x0000_FF00)
        });
        method!(
            builder,
            "rotateEyes",
            [leds],
            |(color, _time, duration): (i32, f32, f32)| {
                wait(duration).await?;
                leds.update("FaceLeds", |state| state.color = color & 0x00FF_FFFF)
            }
        );
        AnyObject::new(builder.build())
    }
}

impl std::fmt::Debug for Leds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Leds")
            .field("groups", &self.groups().len())
            .finish()
    }
}

/// Waits for the duration of a fade, at most ten seconds, unless the call is canceled.
async fn wait(seconds: f32) -> qi::Result<()> {
    let duration = Duration::from_secs_f32(seconds.clamp(0.0, 10.0));
    tokio::select! {
        () = tokio::time::sleep(duration) => Ok(()),
        () = call::cancelled() => Err(Error::CallCanceled),
    }
}
