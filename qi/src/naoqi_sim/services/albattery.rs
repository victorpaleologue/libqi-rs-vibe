//! `ALBattery`: the battery charge.

use super::Context;
use qi::{dynamic::ObjectBuilder, AnyObject};

/// Builds the `ALBattery` service object. The charge is the `BatteryChargeChanged` memory key.
pub fn object(context: &Context) -> AnyObject {
    let memory = context.memory.clone();
    let mut builder = ObjectBuilder::new();
    builder.set_description("ALBattery gives access to the battery charge of the robot.");
    method!(builder, "getBatteryCharge", [memory], |(): ()| {
        Ok(memory.get_i32("BatteryChargeChanged").unwrap_or(0))
    });
    method!(builder, "_setBatteryCharge", [memory], |charge: i32| {
        let charge = charge.clamp(0, 100);
        memory.raise("BatteryChargeChanged", charge);
        memory.insert(
            "Device/SubDeviceList/Battery/Charge/Sensor/Value",
            charge as f32 / 100.0,
        );
        Ok(())
    });
    AnyObject::new(builder.build())
}
