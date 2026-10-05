//! `ALTextToSpeech`, `ALDialog` and `ALSpeechRecognition`.

use crate::common;

use common::{Fixture, TIMEOUT};
use futures::StreamExt;
use qi::naoqi_sim::{AlValue, RobotModel};
use qi::{AnyObject, ObjectExt};
use tokio::time::timeout;

#[tokio::test]
async fn say_records_and_raises_events() {
    let fixture = Fixture::start(RobotModel::Nao).await;
    let memory = fixture.service("ALMemory").await;
    let started: AnyObject = memory
        .call("subscriber", "ALTextToSpeech/TextStarted".to_owned())
        .await
        .unwrap();
    let done: AnyObject = memory
        .call("subscriber", "ALTextToSpeech/TextDone".to_owned())
        .await
        .unwrap();
    let mut started = started.subscribe::<_, i32>("signal").await.unwrap();
    let mut done = done.subscribe::<_, i32>("signal").await.unwrap();
    let mut sentences = fixture
        .simulator
        .memory()
        .subscribe("ALTextToSpeech/CurrentSentence")
        .await
        .unwrap();

    let tts = fixture.service("ALTextToSpeech").await;
    let () = tts.call("say", "Hello world".to_owned()).await.unwrap();
    assert_eq!(fixture.simulator.spoken(), ["Hello world"]);
    assert_eq!(timeout(TIMEOUT, started.next()).await.unwrap(), Some(1));
    assert_eq!(timeout(TIMEOUT, started.next()).await.unwrap(), Some(0));
    assert_eq!(timeout(TIMEOUT, done.next()).await.unwrap(), Some(1));
    assert_eq!(timeout(TIMEOUT, done.next()).await.unwrap(), Some(0));
    assert_eq!(
        timeout(TIMEOUT, sentences.next())
            .await
            .unwrap()
            .unwrap()
            .as_str(),
        Some("Hello world")
    );
    // The last sentence stays readable once said.
    let current: String = fixture.get_data("ALTextToSpeech/CurrentSentence").await;
    assert_eq!(current, "Hello world");

    // Posting works too (the driver uses async<void>("say", ...)).
    tts.post("say", "Second".to_owned()).await;
    common::wait_until("the second sentence", || {
        fixture.simulator.spoken().len() == 2
    })
    .await;
    let history: Vec<String> = tts.call("_history", ()).await.unwrap();
    assert_eq!(history, ["Hello world", "Second"]);

    // Language and volume.
    let language: String = tts.call("getLanguage", ()).await.unwrap();
    assert_eq!(language, "English");
    let () = tts.call("setLanguage", "French".to_owned()).await.unwrap();
    let language: String = tts.call("getLanguage", ()).await.unwrap();
    assert_eq!(language, "French");
    assert!(tts
        .call::<(), _, _>("setLanguage", "Klingon".to_owned())
        .await
        .is_err());
    let languages: Vec<String> = tts.call("getAvailableLanguages", ()).await.unwrap();
    assert!(languages.contains(&"English".to_owned()));
    let () = tts.call("setVolume", 0.3f32).await.unwrap();
    let volume: f32 = tts.call("getVolume", ()).await.unwrap();
    assert_eq!(volume, 0.3);
    let () = tts
        .call("setParameter", ("speed".to_owned(), 80.0f32))
        .await
        .unwrap();
    let speed: f32 = tts.call("getParameter", "speed".to_owned()).await.unwrap();
    assert_eq!(speed, 80.0);
}

#[tokio::test]
async fn dialog_listen_action_flow() {
    // The flow of the driver's `listen` action: load a topic, activate it, subscribe to the
    // result key, then an input matching a rule raises the result.
    let fixture = Fixture::start(RobotModel::Pepper).await;
    let dialog = fixture.service("ALDialog").await;
    let memory = fixture.service("ALMemory").await;
    let language: String = dialog.call("getLanguage", ()).await.unwrap();
    assert_eq!(language, "English");
    let () = dialog
        .call("setLanguage", "French".to_owned())
        .await
        .unwrap();
    let language: String = dialog.call("getLanguage", ()).await.unwrap();
    assert_eq!(language, "French");

    let content = "topic: ~ros_1234 ()\nlanguage: English\nu:(_[ \"yes\" \"no\" ]) $ros_1234/result=$1\nu:(_*) $ros_1234/result=$1\n";
    let topic: String = dialog
        .call("loadTopicContent", content.to_owned())
        .await
        .unwrap();
    assert_eq!(topic, "ros_1234");
    let () = dialog.call("activateTopic", topic.clone()).await.unwrap();
    let () = dialog.call("subscribe", topic.clone()).await.unwrap();
    let () = dialog.call("setFocus", topic.clone()).await.unwrap();
    let subscriber: AnyObject = memory
        .call("subscriber", "ros_1234/result".to_owned())
        .await
        .unwrap();
    let mut results = subscriber.subscribe::<_, AlValue>("signal").await.unwrap();
    let asr = fixture.service("ALSpeechRecognition").await;
    let () = asr.call("_enableFreeSpeechToText", ()).await.unwrap();

    let () = dialog.call("forceInput", "yes".to_owned()).await.unwrap();
    // Both rules match "yes": two events.
    let first = timeout(TIMEOUT, results.next()).await.unwrap().unwrap();
    assert_eq!(first.as_str(), Some("yes"));
    let second = timeout(TIMEOUT, results.next()).await.unwrap().unwrap();
    assert_eq!(second.as_str(), Some("yes"));
    let matched: usize = dialog
        .call::<i32, _, _>("_recognize", "whatever".to_owned())
        .await
        .unwrap() as usize;
    assert_eq!(matched, 1);

    let activated: Vec<String> = dialog.call("getActivatedTopics", ()).await.unwrap();
    assert_eq!(activated, ["ros_1234"]);
    let () = dialog.call("deactivateTopic", topic.clone()).await.unwrap();
    let () = dialog.call("unloadTopic", topic.clone()).await.unwrap();
    assert!(dialog
        .call::<(), _, _>("activateTopic", topic)
        .await
        .is_err());
}

#[tokio::test]
async fn speech_recognition_events() {
    let fixture = Fixture::start(RobotModel::Nao).await;
    let asr = fixture.service("ALSpeechRecognition").await;
    let memory = fixture.service("ALMemory").await;
    let () = asr
        .call(
            "setVocabulary",
            (vec!["hello".to_owned(), "bye".to_owned()], false),
        )
        .await
        .unwrap();
    // Not running yet.
    assert!(asr
        .call::<(), _, _>("_recognize", ("hello".to_owned(), 0.9f32))
        .await
        .is_err());
    let () = asr.call("subscribe", "Test".to_owned()).await.unwrap();
    let subscriber: AnyObject = memory
        .call("subscriber", "WordRecognized".to_owned())
        .await
        .unwrap();
    let mut words = subscriber.subscribe::<_, AlValue>("signal").await.unwrap();
    let () = asr
        .call("_recognize", ("hello".to_owned(), 0.9f32))
        .await
        .unwrap();
    let word = timeout(TIMEOUT, words.next()).await.unwrap().unwrap();
    let items = word.as_list().unwrap();
    assert_eq!(items[0].as_str(), Some("hello"));
    assert!((items[1].as_f32().unwrap() - 0.9).abs() < 1e-6);
    // Unknown words are refused without free speech.
    assert!(asr
        .call::<(), _, _>("_recognize", ("banana".to_owned(), 0.9f32))
        .await
        .is_err());
    let () = asr.call("pause", true).await.unwrap();
    let running: bool = asr.call("isRunning", ()).await.unwrap();
    assert!(!running);
    let () = asr.call("unsubscribe", "Test".to_owned()).await.unwrap();
}
