//! The initial `ALMemory` keys of a robot: every key `naoqi_driver2` reads, with plausible
//! values, plus the state keys of the simulated sensors and services.

use super::{RobotDescription, RobotModel};
use crate::naoqi_sim::alvalue::AlValue;

/// The names of the touch and bumper events of NAO, raised with `1.0` (pressed) or `0.0`.
pub const NAO_TOUCH_EVENTS: [&str; 12] = [
    "FrontTactilTouched",
    "MiddleTactilTouched",
    "RearTactilTouched",
    "HandLeftBackTouched",
    "HandLeftLeftTouched",
    "HandLeftRightTouched",
    "HandRightBackTouched",
    "HandRightLeftTouched",
    "HandRightRightTouched",
    "LeftBumperPressed",
    "RightBumperPressed",
    "ChestButtonPressed",
];

/// The touch and bumper events Pepper raises in addition to those of NAO.
pub const PEPPER_TOUCH_EVENTS: [&str; 1] = ["BackBumperPressed"];

/// The keys of the inertial unit of the torso, relative to `Device/SubDeviceList/<unit>/`.
const IMU_KEYS: [(&str, f32); 9] = [
    ("AngleX", 0.001),
    ("AngleY", -0.002),
    ("AngleZ", 0.0),
    ("GyroscopeX", 0.0),
    ("GyroscopeY", 0.0),
    ("GyroscopeZ", 0.0),
    ("AccelerometerX", 0.05),
    ("AccelerometerY", -0.03),
    ("AccelerometerZ", -9.81),
];

/// The initial keys of the memory of a robot, with the given NAOqi version and robot name.
pub fn initial_memory(
    description: &RobotDescription,
    version: &str,
    robot_name: &str,
) -> Vec<(String, AlValue)> {
    let model = description.model;
    let mut keys: Vec<(String, AlValue)> = Vec::new();
    let mut push = |key: &str, value: AlValue| keys.push((key.to_owned(), value));

    // Identification.
    for (key, value) in description.config_map() {
        push(&key, value.into());
    }
    push("RobotConfig/Head/FullHeadId", "P0000000000ALDR00000".into());
    push(
        "Device/DeviceList/ChestBoard/BodyId",
        "P0000000000ALDR00001".into(),
    );
    push("RobotConfig/Body/SoftwareRequirement", version.into());
    push("RobotConfig/Body/Name", robot_name.into());
    push(
        "Device/DeviceList/ChestBoard/Version",
        model.hardware_version().into(),
    );
    if model == RobotModel::Pepper {
        push(
            "Device/DeviceList/BatteryFuelGauge/SerialNumber",
            "SIM000001".into(),
        );
        push(
            "Device/DeviceList/BatteryFuelGauge/FirmwareVersion",
            "1.2.3".into(),
        );
    }

    // Time.
    push("DCM/Time", 0.into());

    // Inertial units.
    for (key, value) in IMU_KEYS {
        push(
            &format!("Device/SubDeviceList/InertialSensor/{key}/Sensor/Value"),
            value.into(),
        );
    }
    if model == RobotModel::Pepper {
        for (key, value) in IMU_KEYS {
            push(
                &format!("Device/SubDeviceList/InertialSensorBase/{key}/Sensor/Value"),
                value.into(),
            );
        }
    }

    // Battery.
    push("BatteryChargeChanged", 87.into());
    push("BatteryPowerPluggedChanged", false.into());
    push("BatteryFullChargedFlagChanged", false.into());
    push("BatteryLowDetected", false.into());
    push(
        "Device/SubDeviceList/Battery/Current/Sensor/Value",
        (-0.62f32).into(),
    );
    push(
        "Device/SubDeviceList/Battery/Charge/Sensor/Value",
        0.87f32.into(),
    );
    push(
        "Device/SubDeviceList/Battery/Temperature/Sensor/Value",
        29.0f32.into(),
    );

    // Sonars.
    match model {
        RobotModel::Nao => {
            push("Device/SubDeviceList/US/Left/Sensor/Value", 0.85f32.into());
            push("Device/SubDeviceList/US/Right/Sensor/Value", 0.92f32.into());
            push("SonarLeftDetected", 0.85f32.into());
            push("SonarRightDetected", 0.92f32.into());
        }
        RobotModel::Pepper => {
            push(
                "Device/SubDeviceList/Platform/Front/Sonar/Sensor/Value",
                1.25f32.into(),
            );
            push(
                "Device/SubDeviceList/Platform/Back/Sonar/Sensor/Value",
                1.60f32.into(),
            );
        }
    }

    // Laser: 3 heads of 15 segments, points on an arc 2 meters away.
    if description.has_laser {
        for (head, center) in [("Right", -1.6f32), ("Front", 0.0f32), ("Left", 1.6f32)] {
            for segment in 1..=15u32 {
                let angle = center + (segment as f32 - 8.0) * 0.06;
                let (sin, cos) = angle.sin_cos();
                push(
                    &format!("Device/SubDeviceList/Platform/LaserSensor/{head}/Horizontal/Seg{segment:02}/X/Sensor/Value"),
                    (2.0 * cos).into(),
                );
                push(
                    &format!("Device/SubDeviceList/Platform/LaserSensor/{head}/Horizontal/Seg{segment:02}/Y/Sensor/Value"),
                    (2.0 * sin).into(),
                );
            }
        }
    }

    // Touch sensors and bumpers: events and device keys.
    for event in NAO_TOUCH_EVENTS {
        push(event, 0.0f32.into());
    }
    if model == RobotModel::Pepper {
        for event in PEPPER_TOUCH_EVENTS {
            push(event, 0.0f32.into());
        }
    }
    for key in [
        "Head/Touch/Front",
        "Head/Touch/Middle",
        "Head/Touch/Rear",
        "LHand/Touch/Back",
        "LHand/Touch/Left",
        "LHand/Touch/Right",
        "RHand/Touch/Back",
        "RHand/Touch/Left",
        "RHand/Touch/Right",
        "ChestBoard/Button",
    ] {
        push(
            &format!("Device/SubDeviceList/{key}/Sensor/Value"),
            0.0f32.into(),
        );
    }
    match model {
        RobotModel::Nao => {
            for key in [
                "LFoot/Bumper/Left",
                "LFoot/Bumper/Right",
                "RFoot/Bumper/Left",
                "RFoot/Bumper/Right",
            ] {
                push(
                    &format!("Device/SubDeviceList/{key}/Sensor/Value"),
                    0.0f32.into(),
                );
            }
            for foot in ["LFoot", "RFoot"] {
                for fsr in ["FrontLeft", "FrontRight", "RearLeft", "RearRight"] {
                    push(
                        &format!("Device/SubDeviceList/{foot}/FSR/{fsr}/Sensor/Value"),
                        1.3f32.into(),
                    );
                }
                push(
                    &format!("Device/SubDeviceList/{foot}/FSR/TotalWeight/Sensor/Value"),
                    5.2f32.into(),
                );
            }
        }
        RobotModel::Pepper => {
            for key in [
                "Platform/FrontLeft/Bumper",
                "Platform/FrontRight/Bumper",
                "Platform/Back/Bumper",
            ] {
                push(
                    &format!("Device/SubDeviceList/{key}/Sensor/Value"),
                    0.0f32.into(),
                );
            }
        }
    }

    // Speech.
    push("ALTextToSpeech/CurrentSentence", "".into());
    push("ALTextToSpeech/TextStarted", 0.into());
    push("ALTextToSpeech/TextDone", 0.into());
    push(
        "ALTextToSpeech/Status",
        AlValue::list([0.into(), "done".into()]),
    );
    push("WordRecognized", AlValue::list([]));
    push("SpeechDetected", 0.into());
    push("ALSpeechRecognition/Status", "Idle".into());
    push("Dialog/LastInput", "".into());

    // Life and posture.
    push("AutonomousLife/State", "disabled".into());
    push("robotIsWakeUp", true.into());
    push("robotHasFallen", false.into());
    push("ALMotion/Safety/MoveFailed", AlValue::list([]));

    // Audio.
    push("ALAudioDevice/OutputVolume", 50.into());
    push("ALAudioSourceLocalization/SoundLocated", AlValue::list([]));

    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_keys_are_present() {
        let keys = initial_memory(RobotModel::Pepper.description(), "2.9.5.1", "sim");
        let names: Vec<&str> = keys.iter().map(|(key, _)| key.as_str()).collect();
        for expected in [
            "RobotConfig/Body/Type",
            "RobotConfig/Body/BaseVersion",
            "RobotConfig/Head/FullHeadId",
            "Device/DeviceList/ChestBoard/BodyId",
            "RobotConfig/Body/Device/LeftArm/Version",
            "RobotConfig/Body/Device/RightArm/Version",
            "RobotConfig/Body/Device/Hand/Left/Version",
            "RobotConfig/Body/Version",
            "RobotConfig/Body/SoftwareRequirement",
            "RobotConfig/Body/Device/Legs/Version",
            "Device/DeviceList/BatteryFuelGauge/SerialNumber",
            "Device/DeviceList/BatteryFuelGauge/FirmwareVersion",
            "RobotConfig/Body/Device/Platform/Version",
            "RobotConfig/Body/Device/Brakes/Version",
            "RobotConfig/Body/Device/Wheel/Version",
            "DCM/Time",
            "Device/SubDeviceList/InertialSensor/AngleX/Sensor/Value",
            "Device/SubDeviceList/InertialSensorBase/AccelerometerZ/Sensor/Value",
            "BatteryChargeChanged",
            "BatteryPowerPluggedChanged",
            "BatteryFullChargedFlagChanged",
            "Device/SubDeviceList/Battery/Current/Sensor/Value",
            "Device/SubDeviceList/Platform/LaserSensor/Right/Horizontal/Seg01/X/Sensor/Value",
            "Device/SubDeviceList/Platform/LaserSensor/Left/Horizontal/Seg15/Y/Sensor/Value",
            "Device/SubDeviceList/Platform/Front/Sonar/Sensor/Value",
            "Device/SubDeviceList/Platform/Back/Sonar/Sensor/Value",
            "BackBumperPressed",
        ] {
            assert!(names.contains(&expected), "missing key {expected}");
        }
        assert_eq!(
            names
                .iter()
                .filter(|name| name.contains("LaserSensor"))
                .count(),
            90
        );
        // Keys are unique.
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
    }

    #[test]
    fn nao_has_its_sonars() {
        let keys = initial_memory(RobotModel::Nao.description(), "2.8.7.4", "sim");
        let names: Vec<&str> = keys.iter().map(|(key, _)| key.as_str()).collect();
        assert!(names.contains(&"Device/SubDeviceList/US/Left/Sensor/Value"));
        assert!(names.contains(&"Device/SubDeviceList/US/Right/Sensor/Value"));
        assert!(!names.contains(&"BackBumperPressed"));
    }
}
