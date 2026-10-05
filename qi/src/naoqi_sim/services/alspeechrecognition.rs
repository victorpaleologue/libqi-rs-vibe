//! `ALSpeechRecognition`: the speech recognition engine.
//!
//! Recognitions are simulated: `_recognize(word, confidence)` raises the `WordRecognized` and
//! `SpeechDetected` events like the engine would after hearing the word.

use super::Context;
use crate::naoqi_sim::{alvalue::AlValue, error, lock};
use qi::{dynamic::ObjectBuilder, AnyObject};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

/// The state of the speech recognition engine. Cheap to clone: clones share the state.
#[derive(Clone)]
pub struct SpeechRecognition(Arc<Inner>);

struct Inner {
    context: Context,
    language: Mutex<String>,
    vocabulary: Mutex<Vec<String>>,
    word_spotting: AtomicBool,
    subscribers: Mutex<HashSet<String>>,
    paused: AtomicBool,
    free_speech: AtomicBool,
}

impl SpeechRecognition {
    /// Creates the speech recognition engine of a robot.
    pub fn new(context: &Context) -> Self {
        Self(Arc::new(Inner {
            context: context.clone(),
            language: Mutex::new("English".to_owned()),
            vocabulary: Mutex::default(),
            word_spotting: AtomicBool::new(false),
            subscribers: Mutex::default(),
            paused: AtomicBool::new(false),
            free_speech: AtomicBool::new(false),
        }))
    }

    /// The current vocabulary.
    pub fn vocabulary(&self) -> Vec<String> {
        lock(&self.0.vocabulary).clone()
    }

    /// The names of the current subscribers.
    pub fn subscribers(&self) -> Vec<String> {
        let mut names: Vec<String> = lock(&self.0.subscribers).iter().cloned().collect();
        names.sort();
        names
    }

    /// Returns true if the engine is running: subscribed to and not paused.
    pub fn is_running(&self) -> bool {
        !lock(&self.0.subscribers).is_empty() && !self.0.paused.load(Ordering::SeqCst)
    }

    /// Simulates the recognition of a word with a confidence, raising the events of the engine.
    /// Fails if the engine is not running, or if the word is not in the vocabulary (unless free
    /// speech to text is enabled).
    pub fn recognize(&self, word: &str, confidence: f32) -> qi::Result<()> {
        if !self.is_running() {
            return Err(error(
                "ALSpeechRecognition::_recognize\n\tthe engine is not running: subscribe first",
            ));
        }
        let known = lock(&self.0.vocabulary)
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(word));
        if !known && !self.0.free_speech.load(Ordering::SeqCst) {
            return Err(error(format!(
                "ALSpeechRecognition::_recognize\n\tword not in vocabulary: {word}"
            )));
        }
        let memory = &self.0.context.memory;
        self.0.context.logs.info(
            "ALSpeechRecognition",
            format!("recognized \"{word}\" ({confidence:.2})"),
        );
        memory.raise("ALSpeechRecognition/Status", "SpeechDetected");
        memory.raise("SpeechDetected", 1);
        memory.raise(
            "WordRecognized",
            AlValue::list([AlValue::from(word), AlValue::from(confidence)]),
        );
        memory.raise("ALSpeechRecognition/Status", "EndOfProcess");
        memory.raise("SpeechDetected", 0);
        memory.raise("ALSpeechRecognition/Status", "Idle");
        Ok(())
    }

    /// Builds the `ALSpeechRecognition` service object.
    pub fn object(&self) -> AnyObject {
        let asr = self.clone();
        let mut builder = ObjectBuilder::new();
        builder.set_description("ALSpeechRecognition gives the robot the ability to recognize predefined words or phrases in several languages.");
        method!(builder, "setLanguage", [asr], |language: String| {
            if !super::altexttospeech::LANGUAGES.contains(&language.as_str()) {
                return Err(error(format!(
                    "ALSpeechRecognition::setLanguage\n\tunsupported language: {language}"
                )));
            }
            *lock(&asr.0.language) = language;
            Ok(())
        });
        method!(builder, "getLanguage", [asr], |(): ()| {
            Ok(lock(&asr.0.language).clone())
        });
        method!(builder, "getAvailableLanguages", [], |(): ()| {
            Ok(super::altexttospeech::LANGUAGES
                .iter()
                .map(|language| (*language).to_owned())
                .collect::<Vec<String>>())
        });
        method!(
            builder,
            "setVocabulary",
            [asr],
            |(words, word_spotting): (Vec<String>, bool)| {
                if words.is_empty() {
                    return Err(error(
                        "ALSpeechRecognition::setVocabulary\n\tthe vocabulary is empty",
                    ));
                }
                asr.0.context.logs.info(
                    "ALSpeechRecognition",
                    format!("vocabulary set: {}", words.join(", ")),
                );
                *lock(&asr.0.vocabulary) = words;
                asr.0.word_spotting.store(word_spotting, Ordering::SeqCst);
                Ok(())
            }
        );
        method!(builder, "subscribe", [asr], |name: String| {
            asr.0
                .context
                .logs
                .info("ALSpeechRecognition", format!("{name} subscribed"));
            lock(&asr.0.subscribers).insert(name);
            asr.0
                .context
                .memory
                .raise("ALSpeechRecognition/Status", "Idle");
            Ok(())
        });
        method!(builder, "unsubscribe", [asr], |name: String| {
            if lock(&asr.0.subscribers).remove(&name) {
                Ok(())
            } else {
                Err(error(format!(
                    "ALSpeechRecognition::unsubscribe\n\tunknown subscriber: {name}"
                )))
            }
        });
        method!(builder, "pause", [asr], |paused: bool| {
            asr.0.paused.store(paused, Ordering::SeqCst);
            Ok(())
        });
        method!(builder, "setAudioExpression", [asr], |enabled: bool| {
            asr.0
                .context
                .memory
                .insert("ALSpeechRecognition/AudioExpression", enabled);
            Ok(())
        });
        method!(builder, "setVisualExpression", [asr], |enabled: bool| {
            asr.0
                .context
                .memory
                .insert("ALSpeechRecognition/VisualExpression", enabled);
            Ok(())
        });
        method!(builder, "getSubscribersInfo", [asr], |(): ()| {
            Ok(AlValue::list(
                asr.subscribers()
                    .into_iter()
                    .map(|name| AlValue::list([AlValue::from(name)])),
            ))
        });
        method!(builder, "isRunning", [asr], |(): ()| {
            Ok(asr.is_running())
        });
        method!(builder, "_enableFreeSpeechToText", [asr], |(): ()| {
            asr.0.free_speech.store(true, Ordering::SeqCst);
            Ok(())
        });
        method!(builder, "_disableFreeSpeechToText", [asr], |(): ()| {
            asr.0.free_speech.store(false, Ordering::SeqCst);
            Ok(())
        });
        method!(builder, "_recognize", [asr], |(word, confidence): (
            String,
            f32
        )| {
            asr.recognize(&word, confidence)
        });
        AnyObject::new(builder.build())
    }
}

impl std::fmt::Debug for SpeechRecognition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpeechRecognition")
            .field("running", &self.is_running())
            .field("vocabulary", &self.vocabulary().len())
            .finish_non_exhaustive()
    }
}
