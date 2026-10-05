//! The audio extraction path: the simulator calls back the `processRemote` method of a service
//! registered by the client, like `naoqi_driver2` registers `ROS-Driver-Audio`.

use crate::common;

use common::{wait_until, Fixture, TIMEOUT};
use qi::naoqi_sim::RobotModel;
use qi::{
    dynamic::ObjectBuilder,
    value::{AsRaw, Dynamic, Value},
    AnyObject, ObjectExt,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// A received buffer: channels, samples per channel, timestamp and buffer size.
type Received = (i32, i32, Value<'static>, usize);

/// Builds the audio subscriber object of the driver: `processRemote(iimm)`.
fn audio_subscriber(received: Arc<Mutex<Vec<Received>>>) -> AnyObject {
    let mut builder = ObjectBuilder::new();
    builder.add_method(
        "processRemote",
        move |(channels, samples, timestamp, buffer): (
            i32,
            i32,
            Dynamic<Value<'static>>,
            Dynamic<AsRaw<Vec<u8>>>,
        )| {
            let received = received.clone();
            async move {
                received
                    .lock()
                    .unwrap()
                    .push((channels, samples, timestamp.0, buffer.0 .0.len()));
                Ok(())
            }
        },
    );
    AnyObject::new(builder.build())
}

#[tokio::test]
async fn subscriber_receives_pcm_buffers() {
    let simulator =
        qi::naoqi_sim::Simulator::start(qi::naoqi_sim::Config::new(RobotModel::Nao).on_loopback())
            .await
            .unwrap();
    // The client listens so that the simulator can call it back, like the driver does.
    let mut init = qi::node::init();
    init.bind("tcp://127.0.0.1:0".parse().unwrap());
    let client = init
        .connect_to_space(simulator.address().unwrap(), None)
        .start()
        .await
        .unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let service_id = client
        .register_service_object("ROS-Driver-Audio", audio_subscriber(received.clone()))
        .await
        .unwrap();

    let audio = client.service("ALAudioDevice").await.unwrap();
    let () = audio
        .call(
            "setClientPreferences",
            ("ROS-Driver-Audio".to_owned(), 48000, 0, 0),
        )
        .await
        .unwrap();
    let () = audio
        .call("subscribe", "ROS-Driver-Audio".to_owned())
        .await
        .unwrap();
    assert_eq!(
        simulator.services().audio.subscribers(),
        ["ROS-Driver-Audio"]
    );

    wait_until("two audio buffers", || received.lock().unwrap().len() >= 2).await;
    {
        let received = received.lock().unwrap();
        let (channels, samples, timestamp, size) = &received[0];
        assert_eq!(*channels, 4);
        assert_eq!(*samples, 8192);
        assert_eq!(*size, 2 * 4 * 8192);
        // The timestamp is an ALValue [seconds, microseconds].
        match timestamp {
            Value::List(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(items[0], Value::Int32(secs) if secs > 0));
            }
            other => panic!("unexpected timestamp {other}"),
        }
    }

    // Unsubscribing stops the buffers.
    let () = audio
        .call("unsubscribe", "ROS-Driver-Audio".to_owned())
        .await
        .unwrap();
    assert!(simulator.services().audio.subscribers().is_empty());
    let count = received.lock().unwrap().len();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(received.lock().unwrap().len(), count);
    let err = audio
        .call::<(), _, _>("unsubscribe", "ROS-Driver-Audio".to_owned())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown subscriber"), "{err}");
    client.unregister_service(service_id).await.unwrap();
}

#[tokio::test]
async fn mono_16k_preferences_are_honored() {
    let simulator = qi::naoqi_sim::Simulator::start(
        qi::naoqi_sim::Config::new(RobotModel::Pepper).on_loopback(),
    )
    .await
    .unwrap();
    let mut init = qi::node::init();
    init.bind("tcp://127.0.0.1:0".parse().unwrap());
    let client = init
        .connect_to_space(simulator.address().unwrap(), None)
        .start()
        .await
        .unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    client
        .register_service_object("MyAudio", audio_subscriber(received.clone()))
        .await
        .unwrap();
    let audio = client.service("ALAudioDevice").await.unwrap();
    let () = audio
        .call("setClientPreferences", ("MyAudio".to_owned(), 16000, 3, 1))
        .await
        .unwrap();
    let () = audio.call("subscribe", "MyAudio".to_owned()).await.unwrap();
    tokio::time::timeout(TIMEOUT, async {
        while received.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let (channels, samples, _, size) = received.lock().unwrap()[0].clone();
    assert_eq!(channels, 1);
    assert_eq!(samples, 2730);
    assert_eq!(size, 2 * 2730);
    // Unsupported preferences are refused.
    assert!(audio
        .call::<(), _, _>("setClientPreferences", ("MyAudio".to_owned(), 44100, 0, 0))
        .await
        .is_err());
}

#[tokio::test]
async fn subscription_to_a_missing_service_gives_up() {
    let fixture = Fixture::start(RobotModel::Nao).await;
    let audio = fixture.service("ALAudioDevice").await;
    let () = audio.call("subscribe", "Nobody".to_owned()).await.unwrap();
    assert_eq!(fixture.simulator.services().audio.subscribers(), ["Nobody"]);
    // The streaming task keeps trying for a while, then gives up on its own.
    let volume: i32 = audio.call("getOutputVolume", ()).await.unwrap();
    assert_eq!(volume, 50);
    let () = audio.call("setOutputVolume", 30).await.unwrap();
    let volume: i32 = audio.call("getOutputVolume", ()).await.unwrap();
    assert_eq!(volume, 30);
    let () = audio
        .call("unsubscribe", "Nobody".to_owned())
        .await
        .unwrap();
    assert!(fixture.simulator.services().audio.subscribers().is_empty());
}
