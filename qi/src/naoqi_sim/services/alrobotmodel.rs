//! `ALRobotModel`: the hardware description of the robot (NAOqi 2.5 and later).

use super::Context;
use crate::naoqi_sim::robot::RobotModel;
use qi::{dynamic::ObjectBuilder, AnyObject};
use std::collections::HashMap;

/// Builds the `ALRobotModel` service object.
pub fn object(context: &Context) -> AnyObject {
    let description = context.body.description();
    let mut builder = ObjectBuilder::new();
    builder.set_description("ALRobotModel describes the hardware of the robot.");
    method!(builder, "getRobotType", [description], |(): ()| {
        Ok(description.model.body_type().to_owned())
    });
    method!(builder, "hasLegs", [description], |(): ()| {
        Ok(description.has_legs)
    });
    method!(builder, "hasWheels", [description], |(): ()| {
        Ok(description.model == RobotModel::Pepper)
    });
    method!(builder, "hasArms", [], |(): ()| { Ok(true) });
    method!(builder, "hasTablet", [description], |(): ()| {
        Ok(description.model == RobotModel::Pepper)
    });
    method!(builder, "hasLaser", [description], |(): ()| {
        Ok(description.has_laser)
    });
    method!(builder, "hasDepthCamera", [description], |(): ()| {
        Ok(description.cameras.contains(&2))
    });
    method!(builder, "getConfig", [description], |(): ()| {
        Ok(description.config_xml())
    });
    method!(builder, "_getConfigMap", [description], |(): ()| {
        Ok(description.config_hash_map() as HashMap<String, String>)
    });
    method!(builder, "_getMicrophoneConfig", [], |(): ()| { Ok(1) });
    method!(builder, "getRobotVersion", [description], |(): ()| {
        Ok(description.model.hardware_version().to_owned())
    });
    AnyObject::new(builder.build())
}
