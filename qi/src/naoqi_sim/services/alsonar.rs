//! `ALSonar`: the ultrasonic sensors extractor.

use super::Context;
use crate::naoqi_sim::{alvalue::AlValue, lock, robot::RobotModel};
use qi::{dynamic::ObjectBuilder, AnyObject};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// The state of the sonar extractor. Cheap to clone: clones share the state.
#[derive(Clone)]
pub struct Sonar(Arc<Inner>);

struct Inner {
    context: Context,
    subscribers: Mutex<HashMap<String, (i32, f32)>>,
}

impl Sonar {
    /// Creates the sonar extractor of a robot.
    pub fn new(context: &Context) -> Self {
        Self(Arc::new(Inner {
            context: context.clone(),
            subscribers: Mutex::default(),
        }))
    }

    /// The names of the current subscribers.
    pub fn subscribers(&self) -> Vec<String> {
        let mut names: Vec<String> = lock(&self.0.subscribers).keys().cloned().collect();
        names.sort();
        names
    }

    /// The memory keys of the sonar values.
    pub fn keys(&self) -> Vec<&'static str> {
        match self.0.context.robot {
            RobotModel::Nao => vec![
                "Device/SubDeviceList/US/Left/Sensor/Value",
                "Device/SubDeviceList/US/Right/Sensor/Value",
            ],
            RobotModel::Pepper => vec![
                "Device/SubDeviceList/Platform/Front/Sonar/Sensor/Value",
                "Device/SubDeviceList/Platform/Back/Sonar/Sensor/Value",
            ],
        }
    }

    /// Builds the `ALSonar` service object.
    pub fn object(&self) -> AnyObject {
        let sonar = self.clone();
        let mut builder = ObjectBuilder::new();
        builder.set_description(
            "ALSonar retrieves the ultrasonic sensor values and raises the sonar events.",
        );
        method!(builder, "subscribe", [sonar], |name: String| {
            sonar
                .0
                .context
                .logs
                .info("ALSonar", format!("{name} subscribed"));
            lock(&sonar.0.subscribers).insert(name, (100, 0.0));
            Ok(())
        });
        method!(builder, "unsubscribe", [sonar], |name: String| {
            lock(&sonar.0.subscribers).remove(&name);
            sonar
                .0
                .context
                .logs
                .info("ALSonar", format!("{name} unsubscribed"));
            Ok(())
        });
        method!(builder, "updatePeriod", [sonar], |(name, period): (
            String,
            i32
        )| {
            if let Some(entry) = lock(&sonar.0.subscribers).get_mut(&name) {
                entry.0 = period;
            }
            Ok(())
        });
        method!(
            builder,
            "updatePrecision",
            [sonar],
            |(name, precision): (String, f32)| {
                if let Some(entry) = lock(&sonar.0.subscribers).get_mut(&name) {
                    entry.1 = precision;
                }
                Ok(())
            }
        );
        method!(builder, "getSubscribersInfo", [sonar], |(): ()| {
            Ok(AlValue::list(lock(&sonar.0.subscribers).iter().map(
                |(name, (period, precision))| {
                    AlValue::list([
                        AlValue::from(name.as_str()),
                        AlValue::from(*period),
                        AlValue::from(*precision),
                    ])
                },
            )))
        });
        method!(builder, "getCurrentPeriod", [sonar], |(): ()| {
            Ok(lock(&sonar.0.subscribers)
                .values()
                .map(|(period, _)| *period)
                .min()
                .unwrap_or(100))
        });
        method!(builder, "getCurrentPrecision", [], |(): ()| { Ok(0.0f32) });
        method!(builder, "getMyPeriod", [sonar], |name: String| {
            Ok(lock(&sonar.0.subscribers)
                .get(&name)
                .map(|(period, _)| *period)
                .unwrap_or(0))
        });
        method!(builder, "getMyPrecision", [sonar], |name: String| {
            Ok(lock(&sonar.0.subscribers)
                .get(&name)
                .map(|(_, precision)| *precision)
                .unwrap_or(0.0))
        });
        method!(builder, "getOutputNames", [sonar], |(): ()| {
            Ok(sonar
                .keys()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<String>>())
        });
        method!(builder, "getEventList", [], |(): ()| {
            Ok(vec![
                "SonarLeftDetected".to_owned(),
                "SonarRightDetected".to_owned(),
                "SonarLeftNothingDetected".to_owned(),
                "SonarRightNothingDetected".to_owned(),
            ])
        });
        method!(builder, "isRunning", [sonar], |(): ()| {
            Ok(!lock(&sonar.0.subscribers).is_empty())
        });
        AnyObject::new(builder.build())
    }
}

impl std::fmt::Debug for Sonar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sonar")
            .field("subscribers", &self.subscribers())
            .finish()
    }
}
