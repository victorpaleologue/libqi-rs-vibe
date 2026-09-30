//! `ALBodyTemperature`: the temperature diagnosis of the joints.

use super::Context;
use crate::naoqi_sim::alvalue::AlValue;
use qi::{dynamic::ObjectBuilder, AnyObject};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// The temperature above which a joint is reported hot, in degrees Celsius.
const HOT_TEMPERATURE: f32 = 68.0;

/// Builds the `ALBodyTemperature` service object.
pub fn object(context: &Context) -> AnyObject {
    let body = Arc::clone(&context.body);
    let logs = context.logs.clone();
    let notifications = Arc::new(AtomicBool::new(false));
    let mut builder = ObjectBuilder::new();
    builder
        .set_description("ALBodyTemperature monitors the temperature of the joints of the robot.");
    method!(
        builder,
        "setEnableNotifications",
        [notifications, logs],
        |enabled: bool| {
            notifications.store(enabled, Ordering::SeqCst);
            logs.info(
                "ALBodyTemperature",
                format!(
                    "notifications {}",
                    if enabled { "enabled" } else { "disabled" }
                ),
            );
            Ok(())
        }
    );
    method!(
        builder,
        "areNotificationsEnabled",
        [notifications],
        |(): ()| { Ok(notifications.load(Ordering::SeqCst)) }
    );
    method!(builder, "getTemperatureDiagnosis", [body], |(): ()| {
        let indexes: Vec<usize> = (0..body.description().joints.len()).collect();
        let hot: Vec<AlValue> = body
            .temperatures(&indexes)
            .into_iter()
            .zip(body.description().joints)
            .filter(|(temperature, _)| *temperature >= HOT_TEMPERATURE)
            .map(|(_, joint)| AlValue::from(joint.name))
            .collect();
        let level = i32::from(!hot.is_empty());
        Ok(AlValue::list([AlValue::from(level), AlValue::list(hot)]))
    });
    AnyObject::new(builder.build())
}
