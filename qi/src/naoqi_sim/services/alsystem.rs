//! `ALSystem`: the system information of the robot.

use super::Context;
use qi::{dynamic::ObjectBuilder, AnyObject};

/// Builds the `ALSystem` service object.
pub fn object(context: &Context) -> AnyObject {
    let version = context.version.clone();
    let name = context.name.clone();
    let logs = context.logs.clone();
    let mut builder = ObjectBuilder::new();
    builder.set_description("ALSystem provides methods for system management: version, robot name, memory and disk usage.");
    method!(builder, "systemVersion", [version], |(): ()| {
        Ok(version.clone())
    });
    method!(builder, "systemInfo", [version], |(): ()| {
        Ok(vec![
            "naoqi-sim".to_owned(),
            version.clone(),
            "x86_64".to_owned(),
            "Linux".to_owned(),
        ])
    });
    method!(builder, "robotName", [name], |(): ()| { Ok(name.clone()) });
    method!(builder, "setRobotName", [logs], |new_name: String| {
        logs.info(
            "ALSystem",
            format!("robot name change requested: {new_name} (ignored until reboot)"),
        );
        Ok(())
    });
    method!(builder, "timezone", [], |(): ()| {
        Ok("Europe/Paris".to_owned())
    });
    method!(builder, "setTimezone", [logs], |timezone: String| {
        logs.info("ALSystem", format!("timezone set to {timezone}"));
        Ok(())
    });
    method!(builder, "freeMemory", [], |(): ()| { Ok(512 * 1024) });
    method!(builder, "totalMemory", [], |(): ()| { Ok(1024 * 1024) });
    method!(builder, "diskFree", [], |_root: bool| {
        Ok(2 * 1024 * 1024)
    });
    method!(builder, "shutdown", [logs], |(): ()| {
        logs.warn("ALSystem", "shutdown requested (ignored by the simulator)");
        Ok(())
    });
    method!(builder, "reboot", [logs], |(): ()| {
        logs.warn("ALSystem", "reboot requested (ignored by the simulator)");
        Ok(())
    });
    method!(builder, "previousSystemVersion", [version], |(): ()| {
        Ok(version.clone())
    });
    AnyObject::new(builder.build())
}
