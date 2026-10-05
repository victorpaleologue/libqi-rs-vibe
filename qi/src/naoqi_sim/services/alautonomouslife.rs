//! `ALAutonomousLife`: the life state of the robot.

use super::Context;
use crate::naoqi_sim::{error, lock};
use qi::{dynamic::ObjectBuilder, AnyObject};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// The states of the autonomous life.
pub const STATES: [&str; 4] = ["solitary", "interactive", "safeguard", "disabled"];

/// The autonomous abilities.
pub const ABILITIES: [&str; 5] = [
    "AutonomousBlinking",
    "BackgroundMovement",
    "BasicAwareness",
    "ListeningMovement",
    "SpeakingMovement",
];

/// The state of the autonomous life. Cheap to clone: clones share the state.
#[derive(Clone)]
pub struct AutonomousLife(Arc<Inner>);

struct Inner {
    context: Context,
    abilities: Mutex<HashMap<String, bool>>,
}

impl AutonomousLife {
    /// Creates the autonomous life of a robot, disabled.
    pub fn new(context: &Context) -> Self {
        let life = Self(Arc::new(Inner {
            context: context.clone(),
            abilities: Mutex::new(
                ABILITIES
                    .iter()
                    .map(|ability| ((*ability).to_owned(), false))
                    .collect(),
            ),
        }));
        context.memory.insert("AutonomousLife/State", "disabled");
        life
    }

    /// The current state.
    pub fn state(&self) -> String {
        self.0
            .context
            .memory
            .get_string("AutonomousLife/State")
            .unwrap_or_default()
    }

    /// Sets the state, which must be one of [`STATES`].
    pub fn set_state(&self, state: &str) -> qi::Result<()> {
        if !STATES.contains(&state) {
            return Err(error(format!(
                "ALAutonomousLife::setState\n\tunknown state: {state}"
            )));
        }
        self.0
            .context
            .logs
            .info("ALAutonomousLife", format!("state set to {state}"));
        self.0.context.memory.raise("AutonomousLife/State", state);
        Ok(())
    }

    /// Builds the `ALAutonomousLife` service object.
    pub fn object(&self) -> AnyObject {
        let life = self.clone();
        let mut builder = ObjectBuilder::new();
        builder.set_description(
            "ALAutonomousLife manages the life state of the robot and its autonomous abilities.",
        );
        method!(builder, "getState", [life], |(): ()| { Ok(life.state()) });
        method!(builder, "setState", [life], |state: String| {
            life.set_state(&state)
        });
        method!(
            builder,
            "getAutonomousAbilityEnabled",
            [life],
            |ability: String| {
                lock(&life.0.abilities)
                .get(&ability)
                .copied()
                .ok_or_else(|| error(format!("ALAutonomousLife::getAutonomousAbilityEnabled\n\tunknown ability: {ability}")))
            }
        );
        method!(
            builder,
            "setAutonomousAbilityEnabled",
            [life],
            |(ability, enabled): (String, bool)| {
                let mut abilities = lock(&life.0.abilities);
                if ability == "All" {
                    for value in abilities.values_mut() {
                        *value = enabled;
                    }
                    return Ok(());
                }
                match abilities.get_mut(&ability) {
                Some(value) => {
                    *value = enabled;
                    Ok(())
                }
                None => Err(error(format!("ALAutonomousLife::setAutonomousAbilityEnabled\n\tunknown ability: {ability}"))),
            }
            }
        );
        method!(builder, "focusedActivity", [], |(): ()| {
            Ok(String::new())
        });
        method!(builder, "stopFocus", [], |(): ()| { Ok(()) });
        method!(builder, "stopAll", [], |(): ()| { Ok(()) });
        AnyObject::new(builder.build())
    }
}

impl std::fmt::Debug for AutonomousLife {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutonomousLife")
            .field("state", &self.state())
            .finish()
    }
}
