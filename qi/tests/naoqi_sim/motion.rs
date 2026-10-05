//! `ALMotion`: joints, odometry and locomotion.

use crate::common;

use common::{wait_until, Fixture};
use qi::naoqi_sim::{AlValue, RobotModel};
use qi::ObjectExt;
use std::time::Duration;

#[tokio::test]
async fn joints_move_toward_their_targets() {
    let fixture = Fixture::start(RobotModel::Nao).await;
    let motion = fixture.service("ALMotion").await;
    let before: Vec<f32> = motion
        .call("getAngles", ("HeadYaw".to_owned(), true))
        .await
        .unwrap();
    assert_eq!(before, [0.0]);

    // A list of names and angles, like the driver's /joint_angles topic.
    let () = motion
        .call(
            "setAngles",
            (
                vec!["HeadYaw".to_owned(), "HeadPitch".to_owned()],
                vec![0.5f32, 0.2],
                0.5f32,
            ),
        )
        .await
        .unwrap();
    let commanded: Vec<f32> = motion
        .call(
            "getAngles",
            (vec!["HeadYaw".to_owned(), "HeadPitch".to_owned()], false),
        )
        .await
        .unwrap();
    assert_eq!(commanded, [0.5, 0.2]);
    let body = fixture.simulator.body().clone();
    let head = body.resolve(&["Head".to_owned()]).unwrap();
    wait_until("the head to reach its target", || body.reached(&head, 0.01)).await;
    let angles: Vec<f32> = motion
        .call("getAngles", ("Head".to_owned(), true))
        .await
        .unwrap();
    assert!(
        (angles[0] - 0.5).abs() < 0.01 && (angles[1] - 0.2).abs() < 0.01,
        "{angles:?}"
    );
    let sensor: f32 = fixture
        .get_data("Device/SubDeviceList/HeadYaw/Position/Sensor/Value")
        .await;
    assert!((sensor - 0.5).abs() < 0.01, "{sensor}");

    // Relative changes, and clamping to the limits.
    let () = motion
        .call("changeAngles", ("HeadYaw".to_owned(), 10.0f32, 1.0f32))
        .await
        .unwrap();
    let commanded: Vec<f32> = motion
        .call("getAngles", ("HeadYaw".to_owned(), false))
        .await
        .unwrap();
    assert_eq!(commanded, [2.0857]);

    // Blocking interpolation.
    let () = motion
        .call(
            "angleInterpolationWithSpeed",
            ("HeadPitch".to_owned(), 0.0f32, 1.0f32),
        )
        .await
        .unwrap();
    let angles: Vec<f32> = motion
        .call("getAngles", ("HeadPitch".to_owned(), true))
        .await
        .unwrap();
    assert!(angles[0].abs() < 0.01, "{angles:?}");

    // Invalid speeds are errors.
    assert!(motion
        .call::<(), _, _>("setAngles", ("HeadYaw".to_owned(), 0.0f32, 0.0f32))
        .await
        .is_err());
}

#[tokio::test]
async fn stiffness_and_wake_up() {
    let fixture = Fixture::start(RobotModel::Nao).await;
    let motion = fixture.service("ALMotion").await;
    let awake: bool = motion.call("robotIsWakeUp", ()).await.unwrap();
    assert!(awake);
    let () = motion.call("rest", ()).await.unwrap();
    let awake: bool = motion.call("robotIsWakeUp", ()).await.unwrap();
    assert!(!awake);
    let stiffness: Vec<f32> = motion
        .call("getStiffnesses", "Body".to_owned())
        .await
        .unwrap();
    assert!(stiffness.iter().all(|s| *s == 0.0));
    let () = motion.call("wakeUp", ()).await.unwrap();
    let () = motion
        .call("setStiffnesses", ("LArm".to_owned(), 0.5f32))
        .await
        .unwrap();
    let stiffness: Vec<f32> = motion
        .call("getStiffnesses", "LShoulderPitch".to_owned())
        .await
        .unwrap();
    assert_eq!(stiffness, [0.5]);
    let hardness: f32 = fixture
        .get_data("Device/SubDeviceList/LShoulderPitch/Hardness/Actuator/Value")
        .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = hardness;
    let hardness: f32 = fixture
        .get_data("Device/SubDeviceList/LShoulderPitch/Hardness/Actuator/Value")
        .await;
    assert_eq!(hardness, 0.5);
    let summary: String = motion.call("getSummary", ()).await.unwrap();
    assert!(summary.contains("LShoulderPitch"));
}

#[tokio::test]
async fn odometry_follows_move_commands() {
    let fixture = Fixture::start(RobotModel::Pepper).await;
    let motion = fixture.service("ALMotion").await;
    let start: Vec<f32> = motion
        .call("getPosition", ("Torso".to_owned(), 1, true))
        .await
        .unwrap();
    assert_eq!(start.len(), 6);
    assert_eq!(&start[0..2], [0.0, 0.0]);
    assert!(start[2] > 0.5, "Pepper's torso is high: {start:?}");

    let () = motion.call("move", (0.2f32, 0.0f32, 0.0f32)).await.unwrap();
    let velocity: Vec<f32> = motion.call("getRobotVelocity", ()).await.unwrap();
    assert_eq!(velocity, [0.2, 0.0, 0.0]);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let position: Vec<f32> = motion
        .call("getPosition", ("Torso".to_owned(), 1, true))
        .await
        .unwrap();
    assert!(position[0] > 0.03 && position[0] < 0.1, "{position:?}");
    let () = motion.call("stopMove", ()).await.unwrap();
    let velocity: Vec<f32> = motion.call("getRobotVelocity", ()).await.unwrap();
    assert_eq!(velocity, [0.0, 0.0, 0.0]);

    // moveToward uses fractions of the maximum velocity, clamped.
    let () = motion
        .call("moveToward", (2.0f32, 0.0f32, -0.5f32))
        .await
        .unwrap();
    let velocity: Vec<f32> = motion.call("getRobotVelocity", ()).await.unwrap();
    assert_eq!(velocity, [0.55, 0.0, -1.0]);
    let () = motion.call("stopMove", ()).await.unwrap();

    // moveTo blocks until the target is reached, exactly.
    let before = fixture.simulator.body().pose();
    let () = motion
        .call("moveTo", (0.05f32, 0.0f32, 0.3f32))
        .await
        .unwrap();
    let after = fixture.simulator.body().pose();
    assert!((after.x - before.x - 0.05).abs() < 1e-4, "{after:?}");
    assert!((after.theta - before.theta - 0.3).abs() < 1e-4, "{after:?}");
    let moving: bool = motion.call("moveIsActive", ()).await.unwrap();
    assert!(!moving);
    let robot_position: Vec<f32> = motion.call("getRobotPosition", true).await.unwrap();
    assert_eq!(robot_position, [after.x, after.y, after.theta]);
}

#[tokio::test]
async fn limits_and_groups_for_pepper() {
    let fixture = Fixture::start(RobotModel::Pepper).await;
    let motion = fixture.service("ALMotion").await;
    let body: Vec<String> = motion
        .call("getBodyNames", "Body".to_owned())
        .await
        .unwrap();
    assert_eq!(body.len(), 17);
    let actuators: Vec<String> = motion
        .call("getBodyNames", "JointActuators".to_owned())
        .await
        .unwrap();
    assert_eq!(actuators.len(), 20);
    assert!(actuators.contains(&"WheelFL".to_owned()));
    let limits: Vec<Vec<AlValue>> = motion.call("getLimits", "LArm".to_owned()).await.unwrap();
    assert_eq!(limits.len(), 6);
    let wheel_temperature: f32 = fixture
        .get_data("Device/SubDeviceList/WheelFL/Temperature/Sensor/Value")
        .await;
    assert!(wheel_temperature > 20.0);
}

#[tokio::test]
async fn postures_move_the_whole_body() {
    let fixture = Fixture::start(RobotModel::Nao).await;
    let posture = fixture.service("ALRobotPosture").await;
    let family: String = posture.call("getPostureFamily", ()).await.unwrap();
    assert_eq!(family, "Standing");
    let postures: Vec<String> = posture.call("getPostureList", ()).await.unwrap();
    assert!(postures.contains(&"StandZero".to_owned()));
    let reached: bool = posture
        .call("goToPosture", ("StandZero".to_owned(), 1.0f32))
        .await
        .unwrap();
    assert!(reached);
    let motion = fixture.service("ALMotion").await;
    let angles: Vec<f32> = motion
        .call("getAngles", ("LShoulderPitch".to_owned(), true))
        .await
        .unwrap();
    assert!(angles[0].abs() < 0.02, "{angles:?}");
    let current: String = posture.call("getPosture", ()).await.unwrap();
    assert_eq!(current, "StandZero");
    assert!(posture
        .call::<bool, _, _>("goToPosture", ("Fly".to_owned(), 1.0f32))
        .await
        .is_err());
}
