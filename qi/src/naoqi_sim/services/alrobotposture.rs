//! `ALRobotPosture`: the predefined postures of the robot.

use super::Context;
use crate::naoqi_sim::{error, lock};
use qi::{call, dynamic::ObjectBuilder, AnyObject, Error};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// The longest time a posture change waits for the joints.
const POSTURE_TIMEOUT: Duration = Duration::from_secs(20);

/// Builds the `ALRobotPosture` service object.
pub fn object(context: &Context) -> AnyObject {
    let body = Arc::clone(&context.body);
    let logs = context.logs.clone();
    let description = body.description();
    let current: Arc<Mutex<String>> = Arc::new(Mutex::new("Stand".to_owned()));
    let mut builder = ObjectBuilder::new();
    builder.set_description("ALRobotPosture makes the robot go to a predefined posture.");
    method!(builder, "goToPosture", [body, logs, current], |(
        name,
        speed,
    ): (
        String,
        f32
    )| {
        let posture = description.posture(&name).ok_or_else(|| {
            error(format!(
                "ALRobotPosture::goToPosture\n\tunknown posture: {name}"
            ))
        })?;
        logs.info(
            "ALRobotPosture",
            format!("going to posture {name} at speed {speed}"),
        );
        if !body.is_awake() {
            body.wake_up();
        }
        let names: Vec<String> = posture
            .angles
            .iter()
            .map(|(joint, _)| (*joint).to_owned())
            .collect();
        let angles: Vec<f32> = posture.angles.iter().map(|(_, angle)| *angle).collect();
        let indexes = body
            .resolve(&names)
            .map_err(|err| error(format!("ALRobotPosture::goToPosture\n\t{err}")))?;
        body.set_angles(&indexes, &angles, speed.clamp(0.01, 1.0))
            .map_err(|err| error(format!("ALRobotPosture::goToPosture\n\t{err}")))?;
        let reached = tokio::select! {
            reached = body.wait_reached(&indexes, 0.01, POSTURE_TIMEOUT) => reached,
            () = call::cancelled() => return Err(Error::CallCanceled),
        };
        if reached {
            *lock(&current) = name;
        }
        Ok(reached)
    });
    method!(
        builder,
        "applyPosture",
        [body, current],
        |(name, speed): (String, f32)| {
            let posture = description.posture(&name).ok_or_else(|| {
                error(format!(
                    "ALRobotPosture::applyPosture\n\tunknown posture: {name}"
                ))
            })?;
            let names: Vec<String> = posture
                .angles
                .iter()
                .map(|(joint, _)| (*joint).to_owned())
                .collect();
            let angles: Vec<f32> = posture.angles.iter().map(|(_, angle)| *angle).collect();
            let indexes = body
                .resolve(&names)
                .map_err(|err| error(format!("ALRobotPosture::applyPosture\n\t{err}")))?;
            body.set_angles(&indexes, &angles, speed.clamp(0.01, 1.0))
                .map_err(|err| error(format!("ALRobotPosture::applyPosture\n\t{err}")))?;
            *lock(&current) = name;
            Ok(true)
        }
    );
    method!(builder, "stopMove", [body], |(): ()| {
        body.stop_move();
        Ok(())
    });
    method!(builder, "getPostureList", [], |(): ()| {
        Ok(description
            .postures
            .iter()
            .map(|posture| posture.name.to_owned())
            .collect::<Vec<String>>())
    });
    method!(builder, "getPosture", [current], |(): ()| {
        Ok(lock(&current).clone())
    });
    method!(builder, "getPostureFamily", [], |(): ()| {
        Ok(description.posture_family.to_owned())
    });
    method!(builder, "getPostureFamilyList", [], |(): ()| {
        Ok(vec![
            "Standing".to_owned(),
            "Sitting".to_owned(),
            "SittingOnChair".to_owned(),
            "Crouching".to_owned(),
            "LyingBelly".to_owned(),
            "LyingBack".to_owned(),
        ])
    });
    method!(builder, "setMaxTryNumber", [], |_tries: i32| { Ok(()) });
    AnyObject::new(builder.build())
}
