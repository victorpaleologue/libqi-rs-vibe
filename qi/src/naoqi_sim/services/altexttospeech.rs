//! `ALTextToSpeech`: the speech synthesis engine.
//!
//! Utterances are recorded, logged, published as the `ALTextToSpeech/CurrentSentence` key (which
//! keeps the last sentence once said, so that it can be read back) and surrounded by the
//! `ALTextToSpeech/TextStarted` and `ALTextToSpeech/TextDone` events.

use super::Context;
use crate::naoqi_sim::{alvalue::AlValue, error, lock};
use qi::{call, dynamic::ObjectBuilder, AnyObject, Error};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

/// The languages the engine knows.
pub const LANGUAGES: [&str; 9] = [
    "English",
    "French",
    "German",
    "Italian",
    "Japanese",
    "Korean",
    "Portuguese",
    "Spanish",
    "Chinese",
];

/// The state of the speech engine. Cheap to clone: clones share the state.
#[derive(Clone)]
pub struct TextToSpeech(Arc<Inner>);

struct Inner {
    context: Context,
    history: Mutex<Vec<String>>,
    language: Mutex<String>,
    voice: Mutex<String>,
    parameters: Mutex<HashMap<String, f32>>,
    next_id: AtomicU32,
}

impl TextToSpeech {
    /// Creates the speech engine of a robot.
    pub fn new(context: &Context) -> Self {
        let parameters = [
            ("pitchShift", 1.0f32),
            ("doubleVoice", 0.0),
            ("doubleVoiceLevel", 0.0),
            ("doubleVoiceTimeShift", 0.0),
            ("speed", 100.0),
            ("defaultVoiceSpeed", 100.0),
            ("volume", 1.0),
        ];
        Self(Arc::new(Inner {
            context: context.clone(),
            history: Mutex::default(),
            language: Mutex::new("English".to_owned()),
            voice: Mutex::new("naoenu".to_owned()),
            parameters: Mutex::new(
                parameters
                    .into_iter()
                    .map(|(name, value)| (name.to_owned(), value))
                    .collect(),
            ),
            next_id: AtomicU32::new(1),
        }))
    }

    /// Everything the robot said so far, in order.
    pub fn spoken(&self) -> Vec<String> {
        lock(&self.0.history).clone()
    }

    /// The current language.
    pub fn language(&self) -> String {
        lock(&self.0.language).clone()
    }

    /// The current volume, between 0 and 1.
    pub fn volume(&self) -> f32 {
        lock(&self.0.parameters)
            .get("volume")
            .copied()
            .unwrap_or(1.0)
    }

    /// The duration the simulated synthesis of a text takes.
    pub fn duration(text: &str) -> Duration {
        let millis = 40 + 15 * u64::try_from(text.chars().count()).unwrap_or(0);
        Duration::from_millis(millis.min(3000))
    }

    /// Says a text: records it, raises the events and waits for the simulated synthesis.
    pub async fn say(&self, text: &str) -> qi::Result<()> {
        let memory = &self.0.context.memory;
        let id = i32::try_from(self.0.next_id.fetch_add(1, Ordering::Relaxed)).unwrap_or(0);
        lock(&self.0.history).push(text.to_owned());
        self.0
            .context
            .logs
            .info("ALTextToSpeech", format!("say: {text}"));
        memory.insert("ALTextToSpeech/CurrentSentence", text);
        memory.raise(
            "ALTextToSpeech/Status",
            AlValue::list([id.into(), "started".into()]),
        );
        memory.raise("ALTextToSpeech/TextStarted", 1);
        let result = tokio::select! {
            () = tokio::time::sleep(Self::duration(text)) => Ok(()),
            () = call::cancelled() => Err(Error::CallCanceled),
        };
        memory.raise("ALTextToSpeech/TextStarted", 0);
        memory.raise("ALTextToSpeech/TextDone", 1);
        memory.raise(
            "ALTextToSpeech/Status",
            AlValue::list([id.into(), "done".into()]),
        );
        memory.raise("ALTextToSpeech/TextDone", 0);
        result
    }

    /// Builds the `ALTextToSpeech` service object.
    pub fn object(&self) -> AnyObject {
        let tts = self.clone();
        let mut builder = ObjectBuilder::new();
        builder.set_description("This module embeds a speech synthetizer whose role is to convert text commands into sound waves that are then either sent to Nao's loudspeakers or written into a file.");
        method!(builder, "say", [tts], |text: String| {
            tts.say(&text).await
        });
        method!(builder, "sayToFile", [tts], |(text, path): (
            String,
            String
        )| {
            tts.0
                .context
                .logs
                .info("ALTextToSpeech", format!("sayToFile {path}: {text}"));
            lock(&tts.0.history).push(text);
            Ok(())
        });
        method!(builder, "stopAll", [tts], |(): ()| {
            tts.0
                .context
                .memory
                .insert("ALTextToSpeech/CurrentSentence", "");
            Ok(())
        });
        method!(builder, "setLanguage", [tts], |language: String| {
            if !LANGUAGES.contains(&language.as_str()) {
                return Err(error(format!(
                    "ALTextToSpeech::setLanguage\n\tunsupported language: {language}"
                )));
            }
            tts.0
                .context
                .logs
                .info("ALTextToSpeech", format!("language set to {language}"));
            *lock(&tts.0.language) = language;
            Ok(())
        });
        method!(builder, "getLanguage", [tts], |(): ()| {
            Ok(tts.language())
        });
        method!(builder, "getAvailableLanguages", [], |(): ()| {
            Ok(LANGUAGES
                .iter()
                .map(|language| (*language).to_owned())
                .collect::<Vec<String>>())
        });
        method!(builder, "getSupportedLanguages", [], |(): ()| {
            Ok(LANGUAGES
                .iter()
                .map(|language| (*language).to_owned())
                .collect::<Vec<String>>())
        });
        method!(builder, "setVolume", [tts], |volume: f32| {
            lock(&tts.0.parameters).insert("volume".to_owned(), volume.clamp(0.0, 1.0));
            Ok(())
        });
        method!(builder, "getVolume", [tts], |(): ()| { Ok(tts.volume()) });
        method!(builder, "setParameter", [tts], |(name, value): (
            String,
            f32
        )| {
            lock(&tts.0.parameters).insert(name, value);
            Ok(())
        });
        method!(builder, "getParameter", [tts], |name: String| {
            lock(&tts.0.parameters).get(&name).copied().ok_or_else(|| {
                error(format!(
                    "ALTextToSpeech::getParameter\n\tunknown parameter: {name}"
                ))
            })
        });
        method!(builder, "setVoice", [tts], |voice: String| {
            *lock(&tts.0.voice) = voice;
            Ok(())
        });
        method!(builder, "getVoice", [tts], |(): ()| {
            Ok(lock(&tts.0.voice).clone())
        });
        method!(builder, "getAvailableVoices", [], |(): ()| {
            Ok(vec!["naoenu".to_owned(), "naofrf".to_owned()])
        });
        method!(builder, "resetSpeed", [tts], |(): ()| {
            lock(&tts.0.parameters).insert("speed".to_owned(), 100.0);
            Ok(())
        });
        method!(builder, "_history", [tts], |(): ()| { Ok(tts.spoken()) });
        AnyObject::new(builder.build())
    }
}

impl std::fmt::Debug for TextToSpeech {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextToSpeech")
            .field("language", &self.language())
            .field("spoken", &self.spoken().len())
            .finish_non_exhaustive()
    }
}
