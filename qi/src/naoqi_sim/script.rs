//! Scenario scripts: a line-oriented format to drive a running simulator.
//!
//! ```text
//! # Wait 500 ms, then press and release the front head sensor.
//! sleep 500
//! raise FrontTactilTouched 1.0
//! sleep 200
//! raise FrontTactilTouched 0.0
//! insert BatteryChargeChanged 42
//! say "Hello"
//! log something happened
//! ```
//!
//! Commands: `sleep <milliseconds>`, `raise <key> <json>`, `insert <key> <json>` (same as
//! `raise`, kept for readability), `say <text>` and `log <text>`. Blank lines and lines
//! starting with `#` are ignored.

use crate::naoqi_sim::{alvalue::AlValue, simulator::Simulator};
use std::time::Duration;

/// A scenario script.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Script {
    /// The steps of the script, in order.
    pub steps: Vec<Step>,
}

/// A step of a script.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Waits for the duration.
    Sleep(Duration),
    /// Raises a memory event with a value.
    Raise {
        /// The key of the event.
        key: String,
        /// The value of the event.
        value: AlValue,
    },
    /// Makes the robot say a text.
    Say(String),
    /// Emits a log message.
    Log(String),
}

/// The errors of scripts.
#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    /// A line of the script could not be parsed.
    #[error("script line {line}: {message}")]
    Parse {
        /// The 1-based line number.
        line: usize,
        /// What is wrong with the line.
        message: String,
    },
    /// A step failed to run.
    #[error("script step {step} failed: {source}")]
    Run {
        /// The 0-based index of the step.
        step: usize,
        /// The error of the step.
        #[source]
        source: qi::Error,
    },
}

impl Script {
    /// Parses a script.
    pub fn parse(text: &str) -> Result<Self, ScriptError> {
        let mut steps = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (command, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let rest = rest.trim();
            let parse_error = |message: String| ScriptError::Parse {
                line: index + 1,
                message,
            };
            let step = match command {
                "sleep" => {
                    let millis: u64 = rest.parse().map_err(|err| {
                        parse_error(format!("invalid duration \"{rest}\": {err}"))
                    })?;
                    Step::Sleep(Duration::from_millis(millis))
                }
                "raise" | "insert" => {
                    let (key, json) = rest
                        .split_once(char::is_whitespace)
                        .ok_or_else(|| parse_error(format!("expected `{command} <key> <json>`")))?;
                    let json: serde_json::Value = serde_json::from_str(json.trim())
                        .map_err(|err| parse_error(format!("invalid JSON value: {err}")))?;
                    Step::Raise {
                        key: key.to_owned(),
                        value: AlValue::from_json(&json),
                    }
                }
                "say" => Step::Say(unquote(rest)),
                "log" => Step::Log(rest.to_owned()),
                command => return Err(parse_error(format!("unknown command \"{command}\""))),
            };
            steps.push(step);
        }
        Ok(Self { steps })
    }

    /// Returns true if the script has no step.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Runs the script against a simulator.
    pub async fn run(&self, simulator: &Simulator) -> Result<(), ScriptError> {
        for (index, step) in self.steps.iter().enumerate() {
            match step {
                Step::Sleep(duration) => tokio::time::sleep(*duration).await,
                Step::Raise { key, value } => {
                    simulator
                        .logs()
                        .verbose("script", format!("raise {key} = {value}"));
                    simulator.raise(key.clone(), value.clone());
                }
                Step::Say(text) => {
                    simulator.services().tts.say(text).await.map_err(|source| {
                        ScriptError::Run {
                            step: index,
                            source,
                        }
                    })?;
                }
                Step::Log(text) => simulator.logs().info("script", text.clone()),
            }
        }
        Ok(())
    }
}

impl std::str::FromStr for Script {
    type Err = ScriptError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

/// Removes surrounding double quotes, if any.
fn unquote(text: &str) -> String {
    text.strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .unwrap_or(text)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_script() {
        let script: Script = "# comment\n\nsleep 10\nraise FrontTactilTouched 1.0\ninsert Key [1, \"a\"]\nsay \"Hello world\"\nlog done\n"
            .parse()
            .unwrap();
        assert_eq!(script.steps.len(), 5);
        assert_eq!(script.steps[0], Step::Sleep(Duration::from_millis(10)));
        assert_eq!(
            script.steps[1],
            Step::Raise {
                key: "FrontTactilTouched".to_owned(),
                value: AlValue::from(1.0f32),
            }
        );
        assert_eq!(script.steps[3], Step::Say("Hello world".to_owned()));
        assert_eq!(script.steps[4], Step::Log("done".to_owned()));
    }

    #[test]
    fn parse_errors_name_the_line() {
        let err = Script::parse("sleep 10\nfrobnicate\n").unwrap_err();
        assert!(matches!(err, ScriptError::Parse { line: 2, .. }), "{err}");
        let err = Script::parse("raise Key not-json").unwrap_err();
        assert!(matches!(err, ScriptError::Parse { line: 1, .. }), "{err}");
        let err = Script::parse("sleep soon").unwrap_err();
        assert!(matches!(err, ScriptError::Parse { line: 1, .. }), "{err}");
    }
}
