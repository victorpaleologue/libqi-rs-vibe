//! `ALMotion`: joints, stiffness, odometry and locomotion.

use super::Context;
use crate::naoqi_sim::{alvalue::AlValue, body::frame, error, lock};
use qi::{call, dynamic::ObjectBuilder, AnyObject, Error};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

/// The longest time an interpolation waits for its joints.
const INTERPOLATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Builds the `ALMotion` service object.
pub fn object(context: &Context) -> AnyObject {
    let body = Arc::clone(&context.body);
    let logs = context.logs.clone();
    let description = body.description();
    let flags: Arc<Mutex<HashMap<String, bool>>> = Arc::default();
    let mut builder = ObjectBuilder::new();
    builder.set_description(
        "ALMotion module handles the motion of the robot: joints, locomotion and odometry.",
    );

    // Robot description.
    method!(builder, "getRobotConfig", [description], |(): ()| {
        let (names, values) = description.robot_config();
        Ok(vec![names.into_iter().map(AlValue::from).collect(), values] as Vec<Vec<AlValue>>)
    });
    method!(builder, "getSensorNames", [description], |(): ()| {
        Ok(description
            .sensor_names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<String>>())
    });
    method!(builder, "getBodyNames", [description], |name: String| {
        description
            .group_names(&name)
            .ok_or_else(|| error(format!("ALMotion::getBodyNames\n\tunknown name: {name}")))
    });
    method!(builder, "getJointNames", [description], |name: String| {
        description
            .group_names(&name)
            .ok_or_else(|| error(format!("ALMotion::getJointNames\n\tunknown name: {name}")))
    });
    method!(builder, "getLimits", [body], |name: String| {
        let indexes = resolve(&body, &AlValue::from(name), "getLimits")?;
        Ok(body
            .limits(&indexes)
            .into_iter()
            .map(|limits| limits.into_iter().map(AlValue::from).collect())
            .collect::<Vec<Vec<AlValue>>>())
    });
    method!(builder, "getMotionCycleTime", [], |(): ()| { Ok(20) });
    method!(builder, "getSummary", [body], |(): ()| {
        Ok(body.summary())
    });

    // Joints.
    method!(builder, "getAngles", [body], |(names, use_sensors): (
        AlValue,
        bool
    )| {
        let indexes = resolve(&body, &names, "getAngles")?;
        Ok(body.angles(&indexes, use_sensors))
    });
    method!(
        builder,
        "setAngles",
        [body],
        |(names, angles, fraction): (AlValue, AlValue, f32)| {
            let indexes = resolve(&body, &names, "setAngles")?;
            let angles = numbers(&angles, "setAngles", "angles")?;
            body.set_angles(&indexes, &angles, fraction)
                .map_err(|err| error(format!("ALMotion::setAngles\n\t{err}")))
        }
    );
    method!(builder, "changeAngles", [body], |(
        names,
        changes,
        fraction,
    ): (
        AlValue,
        AlValue,
        f32
    )| {
        let indexes = resolve(&body, &names, "changeAngles")?;
        let changes = numbers(&changes, "changeAngles", "changes")?;
        body.change_angles(&indexes, &changes, fraction)
            .map_err(|err| error(format!("ALMotion::changeAngles\n\t{err}")))
    });
    method!(builder, "angleInterpolationWithSpeed", [body], |(
        names,
        angles,
        fraction,
    ): (
        AlValue,
        AlValue,
        f32
    )| {
        let indexes = resolve(&body, &names, "angleInterpolationWithSpeed")?;
        let angles = numbers(&angles, "angleInterpolationWithSpeed", "angles")?;
        body.set_angles(&indexes, &angles, fraction)
            .map_err(|err| error(format!("ALMotion::angleInterpolationWithSpeed\n\t{err}")))?;
        tokio::select! {
            _ = body.wait_reached(&indexes, 0.005, INTERPOLATION_TIMEOUT) => Ok(()),
            () = call::cancelled() => Err(Error::CallCanceled),
        }
    });
    method!(builder, "angleInterpolation", [body], |(
        names,
        angles,
        times,
        _absolute,
    ): (
        AlValue,
        AlValue,
        AlValue,
        bool
    )| {
        let indexes = resolve(&body, &names, "angleInterpolation")?;
        let angles = final_values(&angles, indexes.len(), "angleInterpolation", "angles")?;
        let times = final_values(&times, indexes.len(), "angleInterpolation", "times")?;
        let duration = times.iter().copied().fold(0.0f32, f32::max).max(0.01);
        let current = body.angles(&indexes, true);
        let fraction = indexes
            .iter()
            .zip(&angles)
            .zip(&current)
            .map(|((&index, target), angle)| {
                let max_velocity = body.description().joints[index].max_velocity;
                ((target - angle).abs() / duration / max_velocity).clamp(0.01, 1.0)
            })
            .fold(0.01f32, f32::max);
        body.set_angles(&indexes, &angles, fraction)
            .map_err(|err| error(format!("ALMotion::angleInterpolation\n\t{err}")))?;
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs_f32(duration)) => Ok(()),
            () = call::cancelled() => Err(Error::CallCanceled),
        }
    });
    method!(builder, "getStiffnesses", [body], |names: AlValue| {
        let indexes = resolve(&body, &names, "getStiffnesses")?;
        Ok(body.stiffness(&indexes))
    });
    method!(
        builder,
        "setStiffnesses",
        [body],
        |(names, stiffnesses): (AlValue, AlValue)| {
            let indexes = resolve(&body, &names, "setStiffnesses")?;
            let values = numbers(&stiffnesses, "setStiffnesses", "stiffnesses")?;
            body.set_stiffness(&indexes, &values)
                .map_err(|err| error(format!("ALMotion::setStiffnesses\n\t{err}")))
        }
    );
    method!(builder, "stiffnessInterpolation", [body], |(
        names,
        stiffnesses,
        times,
    ): (
        AlValue,
        AlValue,
        AlValue
    )| {
        let indexes = resolve(&body, &names, "stiffnessInterpolation")?;
        let values = final_values(
            &stiffnesses,
            indexes.len(),
            "stiffnessInterpolation",
            "stiffnesses",
        )?;
        let times = final_values(&times, indexes.len(), "stiffnessInterpolation", "times")?;
        let duration = times.iter().copied().fold(0.0f32, f32::max).max(0.0);
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs_f32(duration)) => {},
            () = call::cancelled() => return Err(Error::CallCanceled),
        }
        body.set_stiffness(&indexes, &values)
            .map_err(|err| error(format!("ALMotion::stiffnessInterpolation\n\t{err}")))
    });

    // Wake up and rest.
    method!(builder, "wakeUp", [body, logs], |(): ()| {
        logs.info("ALMotion", "wakeUp");
        body.wake_up();
        Ok(())
    });
    method!(builder, "rest", [body, logs], |(): ()| {
        logs.info("ALMotion", "rest");
        body.rest();
        Ok(())
    });
    method!(builder, "robotIsWakeUp", [body], |(): ()| {
        Ok(body.is_awake())
    });

    // Cartesian positions and odometry.
    method!(builder, "getPosition", [body], |(
        name,
        frame,
        _use_sensors,
    ): (
        String,
        i32,
        bool
    )| {
        let mut position = body.torso_position(frame);
        match name.as_str() {
            "Torso" => Ok(position.to_vec()),
            "Head" | "CameraTop" | "CameraBottom" => {
                if frame != frame::TORSO {
                    position[2] += 0.16;
                }
                Ok(position.to_vec())
            }
            _ => Err(error(format!(
                "ALMotion::getPosition\n\tunknown name: {name}"
            ))),
        }
    });
    method!(builder, "getRobotPosition", [body], |_use_sensors: bool| {
        let pose = body.pose();
        Ok(vec![pose.x, pose.y, pose.theta])
    });
    method!(builder, "getRobotVelocity", [body], |(): ()| {
        Ok(body.velocity().to_vec())
    });
    method!(builder, "getNextRobotPosition", [body], |(): ()| {
        let pose = body.pose();
        Ok(vec![pose.x, pose.y, pose.theta])
    });

    // Locomotion.
    method!(builder, "move", [body], |(x, y, theta): (f32, f32, f32)| {
        body.set_velocity([x, y, theta]);
        Ok(())
    });
    method!(builder, "moveToward", [body], |(x, y, theta): (
        f32,
        f32,
        f32
    )| {
        body.set_velocity_fraction([x, y, theta]);
        Ok(())
    });
    method!(builder, "moveTo", [body, logs], |(x, y, theta): (
        f32,
        f32,
        f32
    )| {
        logs.info("ALMotion", format!("moveTo x={x} y={y} theta={theta}"));
        let duration = body.move_to(x, y, theta);
        tokio::select! {
            () = tokio::time::sleep(duration + Duration::from_millis(50)) => Ok(()),
            () = call::cancelled() => {
                body.stop_move();
                Err(Error::CallCanceled)
            }
        }
    });
    method!(builder, "stopMove", [body], |(): ()| {
        body.stop_move();
        Ok(())
    });
    method!(builder, "moveInit", [body], |(): ()| {
        body.stop_move();
        Ok(())
    });
    method!(builder, "moveIsActive", [body], |(): ()| {
        Ok(body.is_moving())
    });
    method!(builder, "waitUntilMoveIsFinished", [body], |(): ()| {
        loop {
            if !body.is_moving() {
                return Ok(());
            }
            tokio::select! {
                () = tokio::time::sleep(Duration::from_millis(20)) => {},
                () = call::cancelled() => return Err(Error::CallCanceled),
            }
        }
    });
    method!(builder, "killMove", [body], |(): ()| {
        body.stop_move();
        Ok(())
    });
    method!(builder, "killAll", [body], |(): ()| {
        body.stop_move();
        Ok(())
    });

    // Safety and behavior flags, remembered but without effect.
    method!(
        builder,
        "setExternalCollisionProtectionEnabled",
        [flags],
        |(name, enabled): (String, bool)| {
            lock(&flags).insert(format!("collision/{name}"), enabled);
            Ok(())
        }
    );
    method!(
        builder,
        "getExternalCollisionProtectionEnabled",
        [flags],
        |name: String| {
            Ok(*lock(&flags)
                .get(&format!("collision/{name}"))
                .unwrap_or(&true))
        }
    );
    method!(
        builder,
        "setCollisionProtectionEnabled",
        [flags],
        |(name, enabled): (String, bool)| {
            lock(&flags).insert(format!("armcollision/{name}"), enabled);
            Ok(true)
        }
    );
    method!(
        builder,
        "getCollisionProtectionEnabled",
        [flags],
        |name: String| {
            Ok(*lock(&flags)
                .get(&format!("armcollision/{name}"))
                .unwrap_or(&true))
        }
    );
    method!(builder, "setMoveArmsEnabled", [flags], |(left, right): (
        bool,
        bool
    )| {
        let mut flags = lock(&flags);
        flags.insert("arms/LArm".to_owned(), left);
        flags.insert("arms/RArm".to_owned(), right);
        Ok(())
    });
    method!(builder, "getMoveArmsEnabled", [flags], |name: String| {
        Ok(*lock(&flags).get(&format!("arms/{name}")).unwrap_or(&true))
    });
    method!(builder, "setBreathEnabled", [flags], |(name, enabled): (
        String,
        bool
    )| {
        lock(&flags).insert(format!("breath/{name}"), enabled);
        Ok(())
    });
    method!(builder, "getBreathEnabled", [flags], |name: String| {
        Ok(*lock(&flags)
            .get(&format!("breath/{name}"))
            .unwrap_or(&false))
    });
    method!(builder, "setIdlePostureEnabled", [flags], |(
        name,
        enabled,
    ): (
        String,
        bool
    )| {
        lock(&flags).insert(format!("idle/{name}"), enabled);
        Ok(())
    });
    method!(builder, "getIdlePostureEnabled", [flags], |name: String| {
        Ok(*lock(&flags).get(&format!("idle/{name}")).unwrap_or(&false))
    });
    method!(
        builder,
        "setFallManagerEnabled",
        [flags],
        |enabled: bool| {
            lock(&flags).insert("fallmanager".to_owned(), enabled);
            Ok(())
        }
    );
    method!(builder, "getFallManagerEnabled", [flags], |(): ()| {
        Ok(*lock(&flags).get("fallmanager").unwrap_or(&true))
    });
    method!(
        builder,
        "setSmartStiffnessEnabled",
        [flags],
        |enabled: bool| {
            lock(&flags).insert("smartstiffness".to_owned(), enabled);
            Ok(())
        }
    );
    method!(builder, "getSmartStiffnessEnabled", [flags], |(): ()| {
        Ok(*lock(&flags).get("smartstiffness").unwrap_or(&true))
    });
    AnyObject::new(builder.build())
}

/// Resolves the `names` argument of a method: a joint, chain or group name, or a list of them.
fn resolve(
    body: &crate::naoqi_sim::body::Body,
    names: &AlValue,
    method: &str,
) -> qi::Result<Vec<usize>> {
    let names = names.names().ok_or_else(|| {
        error(format!(
            "ALMotion::{method}\n\tnames must be a string or a list of strings"
        ))
    })?;
    body.resolve(&names)
        .map_err(|err| error(format!("ALMotion::{method}\n\t{err}")))
}

/// Reads a numeric argument: a number or a list of numbers.
fn numbers(value: &AlValue, method: &str, what: &str) -> qi::Result<Vec<f32>> {
    value.numbers().ok_or_else(|| {
        error(format!(
            "ALMotion::{method}\n\t{what} must be a number or a list of numbers"
        ))
    })
}

/// Reads the final values of an interpolation argument: a number, a list of numbers (one per
/// joint), or a list of lists of numbers (a trajectory per joint, whose last value is kept).
fn final_values(value: &AlValue, count: usize, method: &str, what: &str) -> qi::Result<Vec<f32>> {
    let invalid = || error(format!("ALMotion::{method}\n\tinvalid {what}"));
    if let Some(number) = value.as_f32() {
        return Ok(vec![number; count]);
    }
    let items = value.as_list().ok_or_else(invalid)?;
    let values: Vec<f32> = items
        .iter()
        .map(|item| match item.as_f32() {
            Some(number) => Some(number),
            None => item.as_floats().and_then(|floats| floats.last().copied()),
        })
        .collect::<Option<Vec<f32>>>()
        .ok_or_else(invalid)?;
    match values.as_slice() {
        [value] => Ok(vec![*value; count]),
        values if values.len() == count => Ok(values.to_vec()),
        _ => Err(invalid()),
    }
}
