//! The NAOqi services of the simulated robot.
//!
//! Each module implements one service as a dynamic `qi` object whose members carry the exact
//! NAOqi names and signatures. Services share a [`Context`]: the memory, the body, the log hub
//! and the node they are registered on.

use crate::naoqi_sim::{body::Body, log::LogHub, memory::Memory, robot::RobotModel};
use qi::{node::Node, service_directory::LocalServiceDirectory, AnyObject};
use std::sync::{Arc, Weak};

/// Declares a method of a dynamic object from a list of captured handles and an asynchronous
/// body. Each handle is cloned for the method and again for every call.
macro_rules! method {
    ($builder:expr, $name:expr, [$($capture:ident),* $(,)?], |$args:tt : $ty:ty| $body:block) => {{
        $(let $capture = $capture.clone();)*
        $builder.add_method($name, move |$args: $ty| {
            $(let $capture = $capture.clone();)*
            async move $body
        });
    }};
}

pub mod alaudiodevice;
pub mod alautonomouslife;
pub mod albattery;
pub mod albodytemperature;
pub mod aldialog;
pub mod alleds;
pub mod almemory;
pub mod almotion;
pub mod alrobotmodel;
pub mod alrobotposture;
pub mod alsonar;
pub mod alspeechrecognition;
pub mod alsystem;
pub mod altexttospeech;
pub mod alvideodevice;
pub mod logmanager;

/// What the services share.
#[derive(Clone)]
pub struct Context {
    /// The model of the robot.
    pub robot: RobotModel,
    /// The NAOqi version the robot reports.
    pub version: String,
    /// The name of the robot.
    pub name: String,
    /// The memory of the robot.
    pub memory: Memory,
    /// The body of the robot.
    pub body: Arc<Body>,
    /// The log hub of the robot.
    pub logs: LogHub,
    /// The node the services are registered on, used to reach the services of clients.
    pub node: Weak<Node<LocalServiceDirectory>>,
}

impl Context {
    /// The node the services are registered on, if it is still alive.
    pub fn node(&self) -> qi::Result<Arc<Node<LocalServiceDirectory>>> {
        self.node
            .upgrade()
            .ok_or_else(|| crate::naoqi_sim::error("the simulator node is gone"))
    }
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("robot", &self.robot)
            .field("version", &self.version)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// The names of the services of the simulated robot.
pub const SERVICE_NAMES: [&str; 16] = [
    "ALMemory",
    "ALMotion",
    "ALSystem",
    "ALRobotModel",
    "ALVideoDevice",
    "ALAudioDevice",
    "ALBodyTemperature",
    "ALSonar",
    "ALTextToSpeech",
    "ALDialog",
    "ALSpeechRecognition",
    "LogManager",
    "ALLeds",
    "ALBattery",
    "ALAutonomousLife",
    "ALRobotPosture",
];

/// The services of the simulated robot, and the handles to their states.
pub struct Services {
    /// The text to speech engine.
    pub tts: altexttospeech::TextToSpeech,
    /// The audio device.
    pub audio: alaudiodevice::AudioDevice,
    /// The video device.
    pub video: alvideodevice::VideoDevice,
    /// The LEDs.
    pub leds: alleds::Leds,
    /// The dialog engine.
    pub dialog: aldialog::Dialog,
    /// The speech recognition engine.
    pub asr: alspeechrecognition::SpeechRecognition,
    /// The sonars.
    pub sonar: alsonar::Sonar,
    /// The autonomous life.
    pub life: alautonomouslife::AutonomousLife,
    objects: Vec<(&'static str, AnyObject)>,
}

impl Services {
    /// Creates the services of a robot.
    pub fn new(context: &Context) -> Self {
        let tts = altexttospeech::TextToSpeech::new(context);
        let audio = alaudiodevice::AudioDevice::new(context);
        let video = alvideodevice::VideoDevice::new(context);
        let leds = alleds::Leds::new(context);
        let dialog = aldialog::Dialog::new(context);
        let asr = alspeechrecognition::SpeechRecognition::new(context);
        let sonar = alsonar::Sonar::new(context);
        let life = alautonomouslife::AutonomousLife::new(context);
        let objects = vec![
            ("ALMemory", almemory::object(context)),
            ("ALMotion", almotion::object(context)),
            ("ALSystem", alsystem::object(context)),
            ("ALRobotModel", alrobotmodel::object(context)),
            ("ALVideoDevice", video.object()),
            ("ALAudioDevice", audio.object()),
            ("ALBodyTemperature", albodytemperature::object(context)),
            ("ALSonar", sonar.object()),
            ("ALTextToSpeech", tts.object()),
            ("ALDialog", dialog.object()),
            ("ALSpeechRecognition", asr.object()),
            ("LogManager", logmanager::object(context)),
            ("ALLeds", leds.object()),
            ("ALBattery", albattery::object(context)),
            ("ALAutonomousLife", life.object()),
            ("ALRobotPosture", alrobotposture::object(context)),
        ];
        debug_assert_eq!(objects.len(), SERVICE_NAMES.len());
        Self {
            tts,
            audio,
            video,
            leds,
            dialog,
            asr,
            sonar,
            life,
            objects,
        }
    }

    /// The main objects of the services, with their names.
    pub fn objects(&self) -> impl Iterator<Item = (&'static str, &AnyObject)> {
        self.objects.iter().map(|(name, object)| (*name, object))
    }
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services")
            .field("names", &SERVICE_NAMES)
            .finish_non_exhaustive()
    }
}
