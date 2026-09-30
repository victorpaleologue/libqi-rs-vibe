//! `ALMemory`: keys, events and subscribers.

use crate::common;

use common::{Fixture, TIMEOUT};
use futures::StreamExt;
use qi::naoqi_sim::{AlValue, RobotModel};
use qi::{
    value::{Dynamic, Value},
    AnyObject, ObjectExt,
};
use tokio::time::timeout;

#[tokio::test]
async fn subscriber_signals_carry_raised_values() {
    let fixture = Fixture::start(RobotModel::Nao).await;
    let memory = fixture.service("ALMemory").await;
    let subscriber: AnyObject = memory
        .call("subscriber", "FrontTactilTouched".to_owned())
        .await
        .unwrap();
    let mut signal = subscriber.subscribe::<_, AlValue>("signal").await.unwrap();

    // Raised by a client.
    let () = memory
        .call(
            "raiseEvent",
            ("FrontTactilTouched".to_owned(), AlValue::from(1.0f32)),
        )
        .await
        .unwrap();
    let value = timeout(TIMEOUT, signal.next()).await.unwrap().unwrap();
    assert_eq!(value.as_f32(), Some(1.0));

    // Raised in process, with the handle of the simulator (what a scenario does).
    fixture.simulator.touch("FrontTactilTouched", false);
    let value = timeout(TIMEOUT, signal.next()).await.unwrap().unwrap();
    assert_eq!(value.as_f32(), Some(0.0));

    // insertData notifies too, and the value is readable.
    let () = memory
        .call(
            "insertData",
            ("FrontTactilTouched".to_owned(), AlValue::from(1.0f32)),
        )
        .await
        .unwrap();
    let value = timeout(TIMEOUT, signal.next()).await.unwrap().unwrap();
    assert_eq!(value.as_f32(), Some(1.0));
    let stored: f32 = memory
        .call("getData", "FrontTactilTouched".to_owned())
        .await
        .unwrap();
    assert_eq!(stored, 1.0);

    // Two subscribers of the same key both receive the events.
    let other: AnyObject = memory
        .call("subscriber", "FrontTactilTouched".to_owned())
        .await
        .unwrap();
    let mut other_signal = other.subscribe::<_, f32>("signal").await.unwrap();
    fixture.simulator.raise("FrontTactilTouched", 0.0f32);
    assert_eq!(
        timeout(TIMEOUT, signal.next())
            .await
            .unwrap()
            .unwrap()
            .as_f32(),
        Some(0.0)
    );
    assert_eq!(
        timeout(TIMEOUT, other_signal.next()).await.unwrap(),
        Some(0.0)
    );
}

#[tokio::test]
async fn keys_of_any_type_round_trip() {
    let fixture = Fixture::start(RobotModel::Pepper).await;
    let memory = fixture.service("ALMemory").await;
    let () = memory
        .call("insertData", ("Test/Int".to_owned(), 42))
        .await
        .unwrap();
    let () = memory
        .call("insertData", ("Test/String".to_owned(), "hello".to_owned()))
        .await
        .unwrap();
    let () = memory
        .call("insertData", ("Test/Bool".to_owned(), true))
        .await
        .unwrap();
    let () = memory
        .call(
            "insertData",
            (
                "Test/List".to_owned(),
                AlValue::list([
                    AlValue::from(1),
                    AlValue::from("two"),
                    AlValue::from(3.0f32),
                ]),
            ),
        )
        .await
        .unwrap();
    let int: i32 = memory.call("getData", "Test/Int".to_owned()).await.unwrap();
    assert_eq!(int, 42);
    let float: f64 = memory.call("getData", "Test/Int".to_owned()).await.unwrap();
    assert_eq!(float, 42.0);
    let string: String = memory
        .call("getData", "Test/String".to_owned())
        .await
        .unwrap();
    assert_eq!(string, "hello");
    let boolean: bool = memory
        .call("getData", "Test/Bool".to_owned())
        .await
        .unwrap();
    assert!(boolean);
    let Dynamic(list): Dynamic<Value<'static>> = memory
        .call("getData", "Test/List".to_owned())
        .await
        .unwrap();
    let list = AlValue::new(list).as_list().unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[1].as_str(), Some("two"));

    let keys: Vec<String> = memory
        .call("getDataList", "Test/".to_owned())
        .await
        .unwrap();
    assert_eq!(keys, ["Test/Bool", "Test/Int", "Test/List", "Test/String"]);
    let values: Vec<AlValue> = memory
        .call(
            "getListData",
            vec!["Test/Int".to_owned(), "Missing".to_owned()],
        )
        .await
        .unwrap();
    assert_eq!(values[0].as_i32(), Some(42));
    assert_eq!(values[1], AlValue(Value::Unit));
    let ty: String = memory
        .call("getType", "Test/String".to_owned())
        .await
        .unwrap();
    assert_eq!(ty, "String");

    let () = memory
        .call("removeData", "Test/Int".to_owned())
        .await
        .unwrap();
    assert!(memory
        .call::<i32, _, _>("getData", "Test/Int".to_owned())
        .await
        .is_err());
    let () = memory
        .call("declareEvent", "Test/Event".to_owned())
        .await
        .unwrap();
    let events: Vec<String> = memory.call("getEventList", ()).await.unwrap();
    assert!(events.contains(&"Test/Event".to_owned()));
}

#[tokio::test]
async fn legacy_subscribe_to_event_calls_back_a_module() {
    // The NAOqi 1.x pattern: a module registered by the client receives (key, value, message).
    let simulator =
        qi::naoqi_sim::Simulator::start(qi::naoqi_sim::Config::new(RobotModel::Nao).on_loopback())
            .await
            .unwrap();
    let mut init = qi::node::init();
    init.bind("tcp://127.0.0.1:0".parse().unwrap());
    let client = init
        .connect_to_space(simulator.address().unwrap(), None)
        .start()
        .await
        .unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut builder = qi::dynamic::ObjectBuilder::new();
    builder.add_method(
        "onTouched",
        move |(key, value, message): (String, AlValue, String)| {
            let sender = sender.clone();
            async move {
                let _ = sender.send((key, value, message));
                Ok(())
            }
        },
    );
    client
        .register_service_object("MyModule", AnyObject::new(builder.build()))
        .await
        .unwrap();
    let memory = client.service("ALMemory").await.unwrap();
    let () = memory
        .call(
            "subscribeToEvent",
            (
                "RearTactilTouched".to_owned(),
                "MyModule".to_owned(),
                "onTouched".to_owned(),
            ),
        )
        .await
        .unwrap();
    simulator.touch("RearTactilTouched", true);
    let (key, value, message) = timeout(TIMEOUT, receiver.recv()).await.unwrap().unwrap();
    assert_eq!(key, "RearTactilTouched");
    assert_eq!(value.as_f32(), Some(1.0));
    assert_eq!(message, "MyModule");
    let () = memory
        .call(
            "unsubscribeToEvent",
            ("RearTactilTouched".to_owned(), "MyModule".to_owned()),
        )
        .await
        .unwrap();
}
