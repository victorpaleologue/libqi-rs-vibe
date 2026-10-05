//! The startup contract of `naoqi_driver2` (section 11 of its API usage reference), replayed
//! in order against the simulator, for NAO and Pepper.

use crate::common;

use common::Fixture;
use qi::naoqi_sim::{AlValue, LogMessage, RobotModel};
use qi::{
    value::{Dynamic, Value},
    AnyObject, ObjectExt, ServiceDirectory,
};
use std::collections::HashMap;

/// The touch events the driver subscribes to on every robot.
const TOUCH_EVENTS: [&str; 11] = [
    "RightBumperPressed",
    "LeftBumperPressed",
    "HandRightBackTouched",
    "HandRightLeftTouched",
    "HandRightRightTouched",
    "HandLeftBackTouched",
    "HandLeftLeftTouched",
    "HandLeftRightTouched",
    "FrontTactilTouched",
    "MiddleTactilTouched",
    "RearTactilTouched",
];

/// The keys of the `info` converter, read with `getListData`.
const INFO_KEYS: [&str; 10] = [
    "RobotConfig/Head/FullHeadId",
    "Device/DeviceList/ChestBoard/BodyId",
    "RobotConfig/Body/Type",
    "RobotConfig/Body/BaseVersion",
    "RobotConfig/Body/Device/LeftArm/Version",
    "RobotConfig/Body/Device/RightArm/Version",
    "RobotConfig/Body/Device/Hand/Left/Version",
    "RobotConfig/Body/Version",
    "RobotConfig/Body/SoftwareRequirement",
    "RobotConfig/Body/Device/Legs/Version",
];

fn has_method(object: &AnyObject, name: &str) -> bool {
    object
        .meta()
        .methods
        .values()
        .any(|method| method.name == name)
}

fn has_signal(object: &AnyObject, name: &str) -> bool {
    object
        .meta()
        .signals
        .values()
        .any(|signal| signal.name == name)
}

async fn startup_contract(robot: RobotModel, expected_type: &str, expected_version: &str) {
    let fixture = Fixture::start(robot).await;
    let is_pepper = robot == RobotModel::Pepper;

    // 1. The service directory is reachable and lists the services.
    let services = fixture.client.service_directory().services().await.unwrap();
    let names: Vec<&str> = services.iter().map(|info| info.name()).collect();
    for name in qi::naoqi_sim::services::SERVICE_NAMES {
        assert!(names.contains(&name), "missing service {name} in {names:?}");
    }

    // 2-3. Robot identification through ALMemory.
    let memory = fixture.service("ALMemory").await;
    let body_type: String = memory
        .call("getData", "RobotConfig/Body/Type".to_owned())
        .await
        .unwrap();
    assert_eq!(body_type, expected_type);
    let base_version: String = memory
        .call("getData", "RobotConfig/Body/BaseVersion".to_owned())
        .await
        .unwrap();
    assert!(!base_version.is_empty());
    // Type sniffing with a dynamic return value works too.
    let Dynamic(value): Dynamic<Value<'static>> = memory
        .call("getData", "RobotConfig/Body/Type".to_owned())
        .await
        .unwrap();
    assert_eq!(value, Value::String(expected_type.into()));

    // 4. The NAOqi version, made of exactly four numbers.
    let system = fixture.service("ALSystem").await;
    let version: String = system.call("systemVersion", ()).await.unwrap();
    assert_eq!(version, expected_version);
    assert_eq!(version.split('.').count(), 4);
    assert!(version.split('.').all(|part| part.parse::<u32>().is_ok()));

    // 5. The robot configuration: names and values, as a list of lists of dynamic values.
    let motion = fixture.service("ALMotion").await;
    let config: Vec<Vec<AlValue>> = motion.call("getRobotConfig", ()).await.unwrap();
    assert_eq!(config.len(), 2);
    assert_eq!(config[0].len(), config[1].len());
    let index = config[0]
        .iter()
        .position(|name| name.as_str() == Some("Model Type"))
        .expect("Model Type");
    assert!(config[1][index].as_str().is_some());
    let legs = config[0]
        .iter()
        .position(|name| name.as_str() == Some("Number of Legs"))
        .expect("Number of Legs");
    assert_eq!(
        config[1][legs].as_i32(),
        Some(if is_pepper { 0 } else { 2 })
    );
    // The fallback of the driver for NAOqi 2.9.
    let robot_model = fixture.service("ALRobotModel").await;
    let robot_type: String = robot_model.call("getRobotType", ()).await.unwrap();
    assert_eq!(robot_type, expected_type);
    let has_legs: bool = robot_model.call("hasLegs", ()).await.unwrap();
    assert_eq!(has_legs, !is_pepper);
    let xml: String = robot_model.call("getConfig", ()).await.unwrap();
    assert!(xml.contains("<ModulePreference"));
    assert!(xml.contains("memoryName=\"RobotConfig/Head/Version\""));

    // 6. The sensor names.
    let sensors: Vec<String> = motion.call("getSensorNames", ()).await.unwrap();
    assert!(!sensors.is_empty());
    assert_eq!(sensors.iter().any(|name| name == "CameraStereo"), is_pepper);

    // 7. registerDefaultConverter.
    // LogManager and its listener.
    let log_manager = fixture.service("LogManager").await;
    let listener: AnyObject = log_manager.call("getListener", ()).await.unwrap();
    assert!(has_signal(&listener, "onLogMessage"));
    for method in ["setLevel", "addFilter", "clearFilters"] {
        assert!(has_method(&listener, method), "listener lacks {method}");
    }
    let signal = listener
        .meta()
        .signals
        .values()
        .find(|signal| signal.name == "onLogMessage")
        .unwrap();
    assert_eq!(
        signal.signature.to_string(),
        format!(
            "({})",
            qi::value::Signature::from(LogMessage::struct_type())
        )
    );
    // ALBodyTemperature.
    let body_temperature = fixture.service("ALBodyTemperature").await;
    let () = body_temperature
        .call("setEnableNotifications", true)
        .await
        .unwrap();
    // Diagnostics: joint actuators and their limits.
    let actuators: Vec<String> = motion
        .call("getBodyNames", "JointActuators".to_owned())
        .await
        .unwrap();
    assert!(!actuators.is_empty());
    for joint in &actuators {
        let limits: Vec<Vec<AlValue>> = motion.call("getLimits", joint.clone()).await.unwrap();
        assert_eq!(limits.len(), 1, "{joint}");
        assert_eq!(limits[0].len(), 4, "{joint}");
        let values: Vec<f32> = limits[0].iter().map(|v| v.as_f32().unwrap()).collect();
        assert!(values[0] < values[1], "{joint}: {values:?}");
        assert!(values[2] > 0.0, "{joint}: {values:?}");
        // The diagnostics keys of the joint exist.
        for suffix in ["Temperature/Sensor/Value", "Hardness/Actuator/Value"] {
            let value: f32 = fixture
                .get_data(&format!("Device/SubDeviceList/{joint}/{suffix}"))
                .await;
            assert!(value >= 0.0);
        }
    }
    // Cameras.
    let video = fixture.service("ALVideoDevice").await;
    let mut handles = Vec::new();
    for (name, camera, resolution, colorspace) in
        [("front_camera", 0, 1, 11), ("bottom_camera", 1, 1, 11)]
    {
        let handle: String = video
            .call(
                "subscribeCamera",
                (name.to_owned(), camera, resolution, colorspace, 10),
            )
            .await
            .unwrap();
        assert!(handle.starts_with(name), "{handle}");
        handles.push(handle);
    }
    if is_pepper {
        let depth: String = video
            .call("subscribeCamera", ("depth_camera".to_owned(), 2, 9, 17, 10))
            .await
            .unwrap();
        let stereo: String = video
            .call(
                "subscribeCamera",
                ("stereo_camera".to_owned(), 3, 15, 11, 10),
            )
            .await
            .unwrap();
        handles.extend([depth, stereo]);
    }
    // Joint states.
    let body: Vec<String> = motion
        .call("getBodyNames", "Body".to_owned())
        .await
        .unwrap();
    assert!(!body.is_empty());
    // ALSonar (NAOqi < 2.9).
    let sonar = fixture.service("ALSonar").await;
    let () = sonar.call("subscribe", "ROS".to_owned()).await.unwrap();
    // Audio: the microphone configuration, both ways.
    let audio = fixture.service("ALAudioDevice").await;
    let mic: i32 = robot_model.call("_getMicrophoneConfig", ()).await.unwrap();
    assert!(mic >= 0);
    let config_map: HashMap<String, String> = robot_model.call("_getConfigMap", ()).await.unwrap();
    assert!(config_map.contains_key("RobotConfig/Head/Device/Micro/Version"));
    let () = audio
        .call(
            "setClientPreferences",
            ("ROS-Driver-Audio".to_owned(), 48000, 0, 0),
        )
        .await
        .unwrap();
    // Touch subscribers.
    let mut events: Vec<&str> = TOUCH_EVENTS.to_vec();
    if is_pepper {
        events.push("BackBumperPressed");
    }
    for event in events {
        let subscriber: AnyObject = memory.call("subscriber", event.to_owned()).await.unwrap();
        assert!(has_signal(&subscriber, "signal"), "{event}");
        let signal = subscriber
            .meta()
            .signals
            .values()
            .find(|signal| signal.name == "signal")
            .unwrap();
        assert_eq!(signal.signature.to_string(), "(m)", "{event}");
    }

    // 8. registerDefaultSubscriber: ALMotion (already) and ALTextToSpeech.
    let tts = fixture.service("ALTextToSpeech").await;
    assert!(has_method(&tts, "say"));

    // 10. First iteration: joint states, info.
    let position: Vec<f32> = motion
        .call("getPosition", ("Torso".to_owned(), 1, true))
        .await
        .unwrap();
    assert_eq!(position.len(), 6);
    let angles: Vec<f64> = motion
        .call("getAngles", ("Body".to_owned(), true))
        .await
        .unwrap();
    assert_eq!(angles.len(), body.len());
    for joint in &body {
        let velocity: f64 = fixture
            .get_data(&format!("Motion/Velocity/Sensor/{joint}"))
            .await;
        let torque: f64 = fixture
            .get_data(&format!("Motion/Torque/Sensor/{joint}"))
            .await;
        assert!(velocity.is_finite() && torque.is_finite());
    }
    let mut info_keys: Vec<String> = INFO_KEYS.iter().map(|key| (*key).to_owned()).collect();
    if is_pepper {
        info_keys.extend(
            [
                "Device/DeviceList/BatteryFuelGauge/SerialNumber",
                "Device/DeviceList/BatteryFuelGauge/FirmwareVersion",
                "RobotConfig/Body/Device/Platform/Version",
                "RobotConfig/Body/Device/Brakes/Version",
                "RobotConfig/Body/Device/Wheel/Version",
            ]
            .map(str::to_owned),
        );
    }
    let info: Vec<AlValue> = memory.call("getListData", info_keys.clone()).await.unwrap();
    assert_eq!(info.len(), info_keys.len());
    for (key, value) in info_keys.iter().zip(&info) {
        assert!(value.as_str().is_some(), "{key} is not a string: {value}");
    }
    // Per-tick converters: IMU, sonar, diagnostics, laser.
    let imu_keys: Vec<String> = [
        "DCM/Time",
        "Device/SubDeviceList/InertialSensor/AngleX/Sensor/Value",
        "Device/SubDeviceList/InertialSensor/AngleY/Sensor/Value",
        "Device/SubDeviceList/InertialSensor/AngleZ/Sensor/Value",
        "Device/SubDeviceList/InertialSensor/GyroscopeX/Sensor/Value",
        "Device/SubDeviceList/InertialSensor/GyroscopeY/Sensor/Value",
        "Device/SubDeviceList/InertialSensor/GyroscopeZ/Sensor/Value",
        "Device/SubDeviceList/InertialSensor/AccelerometerX/Sensor/Value",
        "Device/SubDeviceList/InertialSensor/AccelerometerY/Sensor/Value",
        "Device/SubDeviceList/InertialSensor/AccelerometerZ/Sensor/Value",
    ]
    .map(str::to_owned)
    .to_vec();
    let imu: Vec<AlValue> = memory.call("getListData", imu_keys.clone()).await.unwrap();
    for (key, value) in imu_keys.iter().zip(&imu) {
        assert!(value.as_f32().is_some(), "{key}: {value}");
    }
    let sonar_keys: Vec<String> = if is_pepper {
        vec![
            "Device/SubDeviceList/Platform/Front/Sonar/Sensor/Value".to_owned(),
            "Device/SubDeviceList/Platform/Back/Sonar/Sensor/Value".to_owned(),
        ]
    } else {
        vec![
            "Device/SubDeviceList/US/Left/Sensor/Value".to_owned(),
            "Device/SubDeviceList/US/Right/Sensor/Value".to_owned(),
        ]
    };
    let sonars: Vec<AlValue> = memory.call("getListData", sonar_keys).await.unwrap();
    assert!(sonars
        .iter()
        .all(|value| value.as_f32().is_some_and(|v| v > 0.0)));
    let battery_keys = [
        "BatteryChargeChanged",
        "BatteryPowerPluggedChanged",
        "BatteryFullChargedFlagChanged",
        "Device/SubDeviceList/Battery/Current/Sensor/Value",
    ]
    .map(str::to_owned)
    .to_vec();
    let battery: Vec<AlValue> = memory.call("getListData", battery_keys).await.unwrap();
    assert!(battery[0]
        .as_i32()
        .is_some_and(|charge| (0..=100).contains(&charge)));
    assert!(battery[1].as_bool().is_some());
    assert!(battery[3].as_f32().is_some());
    if is_pepper {
        let laser_keys: Vec<String> = ["Right", "Front", "Left"]
            .iter()
            .flat_map(|head| {
                (1..=15).flat_map(move |segment| {
                    ["X", "Y"].map(move |axis| {
                        format!("Device/SubDeviceList/Platform/LaserSensor/{head}/Horizontal/Seg{segment:02}/{axis}/Sensor/Value")
                    })
                })
            })
            .collect();
        assert_eq!(laser_keys.len(), 90);
        let laser: Vec<AlValue> = memory.call("getListData", laser_keys).await.unwrap();
        assert_eq!(laser.len(), 90);
        assert!(laser.iter().all(|value| value.as_f32().is_some()));
        let velocity: Vec<f32> = motion.call("getRobotVelocity", ()).await.unwrap();
        assert_eq!(velocity.len(), 3);
    }

    // Shutdown.
    let () = audio
        .call("unsubscribe", "ROS-Driver-Audio".to_owned())
        .await
        .unwrap_or(());
    for handle in handles {
        let unsubscribed: bool = video.call("unsubscribe", handle).await.unwrap();
        assert!(unsubscribed);
    }
    let () = sonar.call("unsubscribe", "ROS".to_owned()).await.unwrap();
}

#[tokio::test]
async fn nao_startup_contract() {
    startup_contract(RobotModel::Nao, "Nao", "2.8.7.4").await;
}

#[tokio::test]
async fn pepper_startup_contract() {
    startup_contract(RobotModel::Pepper, "Pepper", "2.9.5.1").await;
}

#[tokio::test]
async fn missing_keys_and_unknown_names_are_errors() {
    let fixture = Fixture::start(RobotModel::Nao).await;
    let memory = fixture.service("ALMemory").await;
    let err = memory
        .call::<Dynamic<Value<'static>>, _, _>("getData", "Nope/Missing".to_owned())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("key not found"), "{err}");
    let motion = fixture.service("ALMotion").await;
    let err = motion
        .call::<Vec<String>, _, _>("getBodyNames", "Tail".to_owned())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown name"), "{err}");
    let err = motion
        .call::<Vec<f32>, _, _>("getAngles", ("Tail".to_owned(), true))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown"), "{err}");
}

#[tokio::test]
async fn custom_version_and_name() {
    let fixture = Fixture::with_config(
        qi::naoqi_sim::Config::new(RobotModel::Nao)
            .on_loopback()
            .with_version("2.1.4.13")
            .with_name("robby"),
    )
    .await;
    let system = fixture.service("ALSystem").await;
    let version: String = system.call("systemVersion", ()).await.unwrap();
    assert_eq!(version, "2.1.4.13");
    let name: String = system.call("robotName", ()).await.unwrap();
    assert_eq!(name, "robby");
}
