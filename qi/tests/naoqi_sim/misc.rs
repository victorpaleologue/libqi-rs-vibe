//! Authentication, logs, LEDs, battery, sonar, life, and scenario scripts.

use crate::common;

use common::{Fixture, TIMEOUT};
use futures::StreamExt;
use qi::naoqi_sim::{AlValue, Config, LogMessage, RobotModel, Script, Simulator};
use qi::{
    value::{Dynamic, Value},
    AnyObject, ObjectExt,
};
use std::sync::Arc;
use tokio::time::timeout;

#[tokio::test]
async fn authentication_is_enforced_with_a_password() {
    let simulator = Simulator::start(
        Config::new(RobotModel::Nao)
            .on_loopback()
            .with_password("secret"),
    )
    .await
    .unwrap();
    let address = simulator.address().unwrap();

    // Without credentials, the connection is refused.
    assert!(qi::node::init()
        .connect_to_space(address, None)
        .start()
        .await
        .is_err());
    // With a wrong password too.
    let mut credentials = qi::value::KeyDynValueMap::new();
    credentials.set("auth_user", "nao");
    credentials.set("auth_token", "wrong");
    assert!(qi::node::init()
        .connect_to_space(address, Some(credentials))
        .start()
        .await
        .is_err());
    // With the right ones, the robot answers.
    let mut credentials = qi::value::KeyDynValueMap::new();
    credentials.set("auth_user", "nao");
    credentials.set("auth_token", "secret");
    let client = qi::node::init()
        .connect_to_space(address, Some(credentials))
        .start()
        .await
        .unwrap();
    let system = client.service("ALSystem").await.unwrap();
    let version: String = system.call("systemVersion", ()).await.unwrap();
    assert_eq!(version, "2.8.7.4");
}

#[tokio::test]
async fn log_listener_receives_messages() {
    let fixture = Fixture::start(RobotModel::Nao).await;
    let manager = fixture.service("LogManager").await;
    let listener: AnyObject = manager.call("getListener", ()).await.unwrap();
    let mut messages = listener
        .subscribe::<_, LogMessage>("onLogMessage")
        .await
        .unwrap();
    let mut raw = listener
        .subscribe::<_, AlValue>("onLogMessage")
        .await
        .unwrap();
    let level: i32 = listener.property("logLevel").await.unwrap();
    assert_eq!(level, 4);

    // A notable action logs a message.
    let tts = fixture.service("ALTextToSpeech").await;
    let () = tts.call("say", "log me".to_owned()).await.unwrap();
    let message = timeout(TIMEOUT, async {
        loop {
            let message = messages.next().await.unwrap();
            if message.message.contains("log me") {
                return message;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(message.category, "ALTextToSpeech");
    assert_eq!(message.level, 4);
    assert_eq!(message.source.matches(':').count(), 2, "{}", message.source);
    assert!(message.location.contains(':'));
    assert!(message.system_date > 0);
    // The raw value is a tuple of 8 elements, as qicore's LogMessage structure.
    let raw = timeout(TIMEOUT, raw.next()).await.unwrap().unwrap();
    match raw.into_inner() {
        Value::Tuple(elements) => assert_eq!(elements.len(), 8),
        other => panic!("unexpected log message value {other}"),
    }

    // Levels filter the messages: verbose messages are not forwarded by default.
    fixture.simulator.logs().verbose("test", "hidden");
    fixture.simulator.logs().info("test", "shown");
    let message = timeout(TIMEOUT, async {
        loop {
            let message = messages.next().await.unwrap();
            if message.category == "test" {
                return message;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(message.message, "shown");
    let () = listener.call("setLevel", 6).await.unwrap();
    let level: i32 = listener.property("logLevel").await.unwrap();
    assert_eq!(level, 6);
    fixture.simulator.logs().verbose("test", "now visible");
    let message = timeout(TIMEOUT, async {
        loop {
            let message = messages.next().await.unwrap();
            if message.category == "test" {
                return message;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(message.message, "now visible");
    let () = listener
        .call("addFilter", ("test".to_owned(), 2))
        .await
        .unwrap();
    let () = listener.call("clearFilters", ()).await.unwrap();

    // Clients may log through the manager.
    let () = manager
        .call(
            "log",
            vec![LogMessage {
                source: "client.cpp:main:1".to_owned(),
                level: 3,
                category: "client".to_owned(),
                location: "x:1".to_owned(),
                message: "from the client".to_owned(),
                id: 0,
                date: 0,
                system_date: 0,
            }],
        )
        .await
        .unwrap();
    let message = timeout(TIMEOUT, async {
        loop {
            let message = messages.next().await.unwrap();
            if message.category == "client" {
                return message;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(message.message, "from the client");
    assert_eq!(message.level, 3);
}

#[tokio::test]
async fn leds_battery_sonar_and_life() {
    let fixture = Fixture::start(RobotModel::Pepper).await;
    let leds = fixture.service("ALLeds").await;
    let groups: Vec<String> = leds.call("listGroups", ()).await.unwrap();
    assert!(groups.contains(&"FaceLeds".to_owned()));
    let () = leds
        .call("fadeRGB", ("FaceLeds".to_owned(), 0x00FF_0000, 0.0f32))
        .await
        .unwrap();
    let color: i32 = fixture.get_data("ALLeds/FaceLeds/Color").await;
    assert_eq!(color, 0x00FF_0000);
    let () = leds.call("off", "ChestLeds".to_owned()).await.unwrap();
    let intensity: f32 = leds
        .call("getIntensity", "ChestLeds".to_owned())
        .await
        .unwrap();
    assert_eq!(intensity, 0.0);
    let () = leds
        .call("setIntensity", ("ChestLeds".to_owned(), 0.5f32))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .simulator
            .services()
            .leds
            .state("ChestLeds")
            .unwrap()
            .intensity,
        0.5
    );
    assert!(leds
        .call::<(), _, _>("on", "TailLeds".to_owned())
        .await
        .is_err());
    let () = leds
        .call(
            "fadeListRGB",
            (
                "EarLeds".to_owned(),
                vec![0x0000_FF00, 0x0000_00FF],
                vec![0.0f32, 0.0],
            ),
        )
        .await
        .unwrap();
    let color: i32 = fixture.get_data("ALLeds/EarLeds/Color").await;
    assert_eq!(color, 0x0000_00FF);

    let battery = fixture.service("ALBattery").await;
    let charge: i32 = battery.call("getBatteryCharge", ()).await.unwrap();
    assert_eq!(charge, 87);
    fixture.simulator.raise("BatteryChargeChanged", 42);
    let charge: i32 = battery.call("getBatteryCharge", ()).await.unwrap();
    assert_eq!(charge, 42);

    let sonar = fixture.service("ALSonar").await;
    let () = sonar.call("subscribe", "Test".to_owned()).await.unwrap();
    assert_eq!(fixture.simulator.services().sonar.subscribers(), ["Test"]);
    let running: bool = sonar.call("isRunning", ()).await.unwrap();
    assert!(running);
    let outputs: Vec<String> = sonar.call("getOutputNames", ()).await.unwrap();
    assert_eq!(outputs.len(), 2);
    let () = sonar.call("unsubscribe", "Test".to_owned()).await.unwrap();

    let life = fixture.service("ALAutonomousLife").await;
    let state: String = life.call("getState", ()).await.unwrap();
    assert_eq!(state, "disabled");
    let () = life.call("setState", "solitary".to_owned()).await.unwrap();
    let state: String = fixture.get_data("AutonomousLife/State").await;
    assert_eq!(state, "solitary");
    assert!(life
        .call::<(), _, _>("setState", "asleep".to_owned())
        .await
        .is_err());

    let temperature = fixture.service("ALBodyTemperature").await;
    let Dynamic(diagnosis): Dynamic<Value<'static>> = temperature
        .call("getTemperatureDiagnosis", ())
        .await
        .unwrap();
    let diagnosis = AlValue::new(diagnosis).as_list().unwrap();
    assert_eq!(diagnosis[0].as_i32(), Some(0));
}

#[tokio::test]
async fn scripts_drive_the_simulator() {
    let fixture = Fixture::start(RobotModel::Nao).await;
    let memory = fixture.service("ALMemory").await;
    let subscriber: AnyObject = memory
        .call("subscriber", "LeftBumperPressed".to_owned())
        .await
        .unwrap();
    let mut bumper = subscriber.subscribe::<_, f32>("signal").await.unwrap();
    let script: Script = "raise LeftBumperPressed 1.0\nsleep 20\nraise LeftBumperPressed 0.0\nsay \"scripted\"\ninsert Custom/Key {\"a\": 1}\nlog end\n"
        .parse()
        .unwrap();
    fixture.simulator.run_script(&script).await.unwrap();
    assert_eq!(timeout(TIMEOUT, bumper.next()).await.unwrap(), Some(1.0));
    assert_eq!(timeout(TIMEOUT, bumper.next()).await.unwrap(), Some(0.0));
    assert_eq!(fixture.simulator.spoken(), ["scripted"]);
    let Dynamic(custom): Dynamic<Value<'static>> = memory
        .call("getData", "Custom/Key".to_owned())
        .await
        .unwrap();
    assert!(matches!(custom, Value::Map(_)), "{custom}");
}

#[tokio::test]
async fn simulator_shuts_down_cleanly() {
    let simulator = Arc::new(
        Simulator::start(Config::new(RobotModel::Nao).on_loopback())
            .await
            .unwrap(),
    );
    assert_eq!(
        simulator.service_ids().len(),
        qi::naoqi_sim::services::SERVICE_NAMES.len()
    );
    assert!(simulator.uptime() < std::time::Duration::from_secs(5));
    let simulator = Arc::try_unwrap(simulator).ok().unwrap();
    simulator.shutdown().await;
}

#[tokio::test]
async fn tls_endpoints_serve_authenticated_clients() {
    // The path of naoqi_driver2 with a password: tcps:// and the nao user.
    let simulator = Simulator::start(
        Config::new(RobotModel::Pepper)
            .listen_on(vec!["tcps://127.0.0.1:0".parse().unwrap()])
            .with_password("secret"),
    )
    .await
    .unwrap();
    let address = simulator.address().unwrap();
    assert!(address.to_string().starts_with("tcps://"), "{address}");
    let mut credentials = qi::value::KeyDynValueMap::new();
    credentials.set("auth_user", "nao");
    credentials.set("auth_token", "secret");
    let client = qi::node::init()
        .connect_to_space(address, Some(credentials))
        .start()
        .await
        .unwrap();
    let memory = client.service("ALMemory").await.unwrap();
    let robot: String = memory
        .call("getData", "RobotConfig/Body/Type".to_owned())
        .await
        .unwrap();
    assert_eq!(robot, "Pepper");
}
