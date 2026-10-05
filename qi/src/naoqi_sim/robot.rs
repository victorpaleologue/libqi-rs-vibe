//! The descriptions of the simulated robots: joints, chains, sensors, postures and the memory
//! keys they publish.

mod keys;
mod nao;
mod pepper;

pub use keys::{initial_memory, NAO_TOUCH_EVENTS, PEPPER_TOUCH_EVENTS};

use crate::naoqi_sim::alvalue::AlValue;
use qi::value::Value;
use std::{collections::HashMap, fmt::Write as _};

/// The models of robots the simulator knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum RobotModel {
    /// The NAO humanoid (V6 hardware).
    #[default]
    Nao,
    /// The Pepper humanoid (1.8 hardware).
    Pepper,
}

impl RobotModel {
    /// All the models.
    pub const ALL: [Self; 2] = [Self::Nao, Self::Pepper];

    /// The NAOqi version the model runs by default.
    pub fn default_version(self) -> &'static str {
        match self {
            Self::Nao => "2.8.7.4",
            Self::Pepper => "2.9.5.1",
        }
    }

    /// The value of the `RobotConfig/Body/Type` key.
    pub fn body_type(self) -> &'static str {
        match self {
            Self::Nao => "Nao",
            Self::Pepper => "Pepper",
        }
    }

    /// The value of the `RobotConfig/Body/BaseVersion` key.
    pub fn base_version(self) -> &'static str {
        match self {
            Self::Nao => "V6",
            Self::Pepper => "V18",
        }
    }

    /// The version of the hardware parts, as `RobotConfig/*/Version` keys report it.
    pub fn hardware_version(self) -> &'static str {
        match self {
            Self::Nao => "6.0.0",
            Self::Pepper => "1.8.0",
        }
    }

    /// The model type as `ALMotion.getRobotConfig` reports it.
    pub fn model_type(self) -> &'static str {
        match self {
            Self::Nao => "naoH25",
            Self::Pepper => "juliette",
        }
    }

    /// The description of the robot.
    pub fn description(self) -> &'static RobotDescription {
        match self {
            Self::Nao => &nao::DESCRIPTION,
            Self::Pepper => &pepper::DESCRIPTION,
        }
    }

    /// The command-line name of the model.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nao => "nao",
            Self::Pepper => "pepper",
        }
    }
}

impl std::fmt::Display for RobotModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for RobotModel {
    type Err = UnknownRobotModel;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "nao" => Ok(Self::Nao),
            "pepper" | "juliette" => Ok(Self::Pepper),
            _ => Err(UnknownRobotModel(s.to_owned())),
        }
    }
}

/// The error of parsing an unknown robot model name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown robot model \"{0}\", expected \"nao\" or \"pepper\"")]
pub struct UnknownRobotModel(pub String);

/// The kinematic chains of a robot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Chain {
    /// The head.
    Head,
    /// The left arm.
    LArm,
    /// The right arm.
    RArm,
    /// The left leg (NAO).
    LLeg,
    /// The right leg (NAO).
    RLeg,
    /// The leg (Pepper).
    Leg,
    /// The wheels (Pepper).
    Wheels,
}

impl Chain {
    /// The name of the chain in `ALMotion`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Head => "Head",
            Self::LArm => "LArm",
            Self::RArm => "RArm",
            Self::LLeg => "LLeg",
            Self::RLeg => "RLeg",
            Self::Leg => "Leg",
            Self::Wheels => "Wheels",
        }
    }
}

/// The description of a joint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointSpec {
    /// The NAOqi name of the joint.
    pub name: &'static str,
    /// The chain the joint belongs to.
    pub chain: Chain,
    /// The minimum angle, in radians.
    pub min: f32,
    /// The maximum angle, in radians.
    pub max: f32,
    /// The maximum velocity, in radians per second.
    pub max_velocity: f32,
    /// The maximum torque, in newton meters.
    pub max_torque: f32,
    /// The angle of the joint when the simulation starts, in radians.
    pub initial: f32,
    /// Whether the joint has its own actuator (`RHipYawPitch` of NAO has none).
    pub actuator: bool,
    /// Whether the joint is part of the `Body` group (wheels are not).
    pub body: bool,
}

impl JointSpec {
    /// The limits of the joint as `ALMotion.getLimits` reports them.
    pub fn limits(&self) -> [f32; 4] {
        [self.min, self.max, self.max_velocity, self.max_torque]
    }

    /// Clamps an angle to the limits of the joint.
    pub fn clamp(&self, angle: f32) -> f32 {
        angle.clamp(self.min, self.max)
    }
}

/// A predefined posture: the angles of some joints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Posture {
    /// The name of the posture in `ALRobotPosture`.
    pub name: &'static str,
    /// The angles of the joints, by joint name. Joints left out keep their angle.
    pub angles: &'static [(&'static str, f32)],
}

/// The description of a robot model.
#[derive(Debug)]
pub struct RobotDescription {
    /// The model.
    pub model: RobotModel,
    /// The joints, in the order of the `Body` and `JointActuators` groups.
    pub joints: &'static [JointSpec],
    /// The names `ALMotion.getSensorNames` reports.
    pub sensor_names: &'static [&'static str],
    /// The predefined postures.
    pub postures: &'static [Posture],
    /// The family of the postures, as `ALRobotPosture.getPostureFamily` reports it.
    pub posture_family: &'static str,
    /// The height of the torso frame above the ground when standing, in meters.
    pub torso_height: f32,
    /// The maximum velocities of the robot: forward, lateral (m/s) and rotation (rad/s).
    pub max_velocity: [f32; 3],
    /// The groups of LEDs the robot has.
    pub led_groups: &'static [&'static str],
    /// The camera identifiers the robot has.
    pub cameras: &'static [i32],
    /// Whether the robot has legs.
    pub has_legs: bool,
    /// Whether the robot has a laser.
    pub has_laser: bool,
}

impl RobotDescription {
    /// The index of a joint by name.
    pub fn joint_index(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|joint| joint.name == name)
    }

    /// The indexes of the joints of a group: `Body`, `JointActuators`, a chain or a joint name.
    pub fn group(&self, name: &str) -> Option<Vec<usize>> {
        let indexes = |predicate: &dyn Fn(&JointSpec) -> bool| -> Vec<usize> {
            self.joints
                .iter()
                .enumerate()
                .filter(|(_, joint)| predicate(joint))
                .map(|(index, _)| index)
                .collect()
        };
        let chain =
            |chains: &[Chain]| indexes(&|joint| chains.contains(&joint.chain) && joint.body);
        match name {
            "Body" | "Joints" => Some(indexes(&|joint| joint.body)),
            "JointActuators" | "Actuators" => Some(indexes(&|joint| joint.actuator)),
            "Head" => Some(chain(&[Chain::Head])),
            "LArm" => Some(chain(&[Chain::LArm])),
            "RArm" => Some(chain(&[Chain::RArm])),
            "Arms" => Some(chain(&[Chain::LArm, Chain::RArm])),
            "LLeg" => Some(chain(&[Chain::LLeg])),
            "RLeg" => Some(chain(&[Chain::RLeg])),
            "Leg" => Some(chain(&[Chain::Leg])),
            "Legs" => Some(chain(&[Chain::LLeg, Chain::RLeg, Chain::Leg])),
            "Torso" => Some(Vec::new()),
            "Wheels" => Some(indexes(&|joint| joint.chain == Chain::Wheels)),
            name => self.joint_index(name).map(|index| vec![index]),
        }
    }

    /// The names of the joints of a group. See [`RobotDescription::group`].
    pub fn group_names(&self, name: &str) -> Option<Vec<String>> {
        self.group(name).map(|indexes| {
            indexes
                .into_iter()
                .map(|index| self.joints[index].name.to_owned())
                .collect()
        })
    }

    /// The names of the joints of the `Body` group.
    pub fn body_names(&self) -> Vec<String> {
        self.group_names("Body").unwrap_or_default()
    }

    /// Resolves names of joints or groups to joint indexes, in order, without duplicates.
    pub fn resolve(&self, names: &[String]) -> Result<Vec<usize>, String> {
        let mut indexes = Vec::new();
        for name in names {
            let group = self
                .group(name)
                .ok_or_else(|| format!("unknown joint or chain name \"{name}\""))?;
            for index in group {
                if !indexes.contains(&index) {
                    indexes.push(index);
                }
            }
        }
        Ok(indexes)
    }

    /// A predefined posture by name.
    pub fn posture(&self, name: &str) -> Option<&Posture> {
        self.postures.iter().find(|posture| posture.name == name)
    }

    /// The robot configuration as `ALMotion.getRobotConfig` reports it: names and values.
    pub fn robot_config(&self) -> (Vec<String>, Vec<AlValue>) {
        let version = self.model.hardware_version();
        let config: [(&str, AlValue); 12] = [
            ("Model Type", self.model.model_type().into()),
            ("Head Version", version.into()),
            ("Body Version", version.into()),
            ("Arm Version", version.into()),
            ("Laser", self.has_laser.into()),
            ("Legs", self.has_legs.into()),
            ("Arms", true.into()),
            ("Hands", true.into()),
            ("Extended Arms", false.into()),
            ("Number of Legs", if self.has_legs { 2 } else { 0 }.into()),
            ("Number of Arms", 2.into()),
            ("Number of Hands", 2.into()),
        ];
        config
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .unzip()
    }

    /// The robot configuration as `ALRobotModel.getConfig` reports it: an XML document of
    /// module preferences.
    pub fn config_xml(&self) -> String {
        let mut xml = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<ModulePreference schemaLocation=\"ModulePreference.xsd\" xmlns=\"http://www.aldebaran-robotics.com/schema/ModulePreference\">\n",
        );
        for (key, value) in self.config_map() {
            writeln!(
                xml,
                "  <Preference memoryName=\"{key}\" description=\"\" value=\"{value}\" type=\"string\" />"
            )
            .ok();
        }
        xml.push_str("</ModulePreference>\n");
        xml
    }

    /// The robot configuration as `ALRobotModel._getConfigMap` reports it.
    pub fn config_map(&self) -> Vec<(String, String)> {
        let version = self.model.hardware_version();
        let mut map = vec![
            ("RobotConfig/Body/Type", self.model.body_type().to_owned()),
            (
                "RobotConfig/Body/BaseVersion",
                self.model.base_version().to_owned(),
            ),
            ("RobotConfig/Body/Version", version.to_owned()),
            ("RobotConfig/Head/Version", version.to_owned()),
            (
                "RobotConfig/Body/Device/LeftArm/Version",
                version.to_owned(),
            ),
            (
                "RobotConfig/Body/Device/RightArm/Version",
                version.to_owned(),
            ),
            (
                "RobotConfig/Body/Device/Hand/Left/Version",
                version.to_owned(),
            ),
            (
                "RobotConfig/Body/Device/Hand/Right/Version",
                version.to_owned(),
            ),
            ("RobotConfig/Body/Device/Legs/Version", version.to_owned()),
            ("RobotConfig/Head/Device/Micro/Version", "1".to_owned()),
            (
                "RobotConfig/Head/Device/Camera/Top/Version",
                version.to_owned(),
            ),
            (
                "RobotConfig/Head/Device/Camera/Bottom/Version",
                version.to_owned(),
            ),
        ];
        if self.model == RobotModel::Pepper {
            map.extend([
                (
                    "RobotConfig/Body/Device/Platform/Version",
                    version.to_owned(),
                ),
                ("RobotConfig/Body/Device/Brakes/Version", version.to_owned()),
                ("RobotConfig/Body/Device/Wheel/Version", version.to_owned()),
                (
                    "RobotConfig/Head/Device/Camera/Depth/Version",
                    version.to_owned(),
                ),
            ]);
        }
        map.into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect()
    }

    /// The robot configuration as a map, for `ALRobotModel._getConfigMap`.
    pub fn config_hash_map(&self) -> HashMap<String, String> {
        self.config_map().into_iter().collect()
    }

    /// The description of the `Body/Type` key as a value.
    pub fn body_type_value(&self) -> Value<'static> {
        Value::String(self.model.body_type().to_owned().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nao_groups() {
        let nao = RobotModel::Nao.description();
        assert_eq!(nao.body_names().len(), 26);
        assert_eq!(nao.group_names("JointActuators").unwrap().len(), 25);
        assert_eq!(nao.group_names("Head").unwrap(), ["HeadYaw", "HeadPitch"]);
        assert_eq!(nao.group_names("LLeg").unwrap().len(), 6);
        assert!(nao.group_names("Torso").unwrap().is_empty());
        assert!(nao.group_names("Nope").is_none());
        assert_eq!(
            nao.resolve(&["Head".to_owned(), "HeadYaw".to_owned()])
                .unwrap(),
            [0, 1]
        );
        for joint in nao.joints {
            assert!(joint.min < joint.max, "{}", joint.name);
            assert!(
                joint.initial >= joint.min && joint.initial <= joint.max,
                "{}",
                joint.name
            );
        }
    }

    #[test]
    fn pepper_groups() {
        let pepper = RobotModel::Pepper.description();
        assert_eq!(pepper.body_names().len(), 17);
        assert_eq!(pepper.group_names("JointActuators").unwrap().len(), 20);
        assert_eq!(pepper.group_names("Wheels").unwrap().len(), 3);
        assert_eq!(
            pepper.group_names("Leg").unwrap(),
            ["HipRoll", "HipPitch", "KneePitch"]
        );
        assert!(pepper.sensor_names.contains(&"CameraStereo"));
    }

    #[test]
    fn model_names_parse() {
        assert_eq!("NAO".parse::<RobotModel>().unwrap(), RobotModel::Nao);
        assert_eq!("pepper".parse::<RobotModel>().unwrap(), RobotModel::Pepper);
        assert!("romeo".parse::<RobotModel>().is_err());
    }

    #[test]
    fn config_xml_is_parseable_by_the_driver() {
        let xml = RobotModel::Pepper.description().config_xml();
        assert!(xml.contains("<ModulePreference"));
        assert!(xml.contains("memoryName=\"RobotConfig/Head/Version\""));
        assert!(xml.contains("memoryName=\"RobotConfig/Body/Device/LeftArm/Version\""));
    }
}
