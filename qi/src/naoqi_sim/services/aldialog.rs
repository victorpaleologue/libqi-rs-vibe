//! `ALDialog`: the dialog engine, with a minimal understanding of QiChat topics.
//!
//! Topics are loaded from their content, activated and subscribed to. Recognized inputs
//! (`forceInput` or `_recognize`) are matched against the `u:` rules of the activated topics:
//! a rule `u:(_*) $key=$1` or `u:(_[ "a" "b" ]) $key=$1` raises the memory event `key` with the
//! input, which is how `naoqi_driver2` implements its `listen` action.

use super::Context;
use crate::naoqi_sim::{error, lock};
use qi::{dynamic::ObjectBuilder, AnyObject};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

/// A loaded topic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topic {
    /// The name of the topic, from its `topic:` line.
    pub name: String,
    /// The language of the topic, from its `language:` line.
    pub language: String,
    /// The rules of the topic: the accepted inputs (`None` for any) and the variable to set.
    pub rules: Vec<Rule>,
    /// Whether the topic is activated.
    pub active: bool,
}

/// A `u:` rule of a topic that assigns a variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The inputs the rule accepts, lower-cased; `None` accepts anything.
    pub inputs: Option<Vec<String>>,
    /// The memory key the rule sets to the input.
    pub variable: String,
}

impl Topic {
    /// Parses the content of a topic file.
    pub fn parse(content: &str) -> Result<Self, String> {
        let mut name = None;
        let mut language = "English".to_owned();
        let mut rules = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("topic:") {
                let rest = rest.trim().trim_start_matches('~');
                let topic_name: String = rest
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != '(')
                    .collect();
                if topic_name.is_empty() {
                    return Err("empty topic name".to_owned());
                }
                name = Some(topic_name);
            } else if let Some(rest) = line.strip_prefix("language:") {
                language = rest.trim().to_owned();
            } else if let Some(rest) = line.strip_prefix("u:") {
                if let Some(rule) = Rule::parse(rest) {
                    rules.push(rule);
                }
            }
        }
        Ok(Self {
            name: name.ok_or_else(|| "missing topic: line".to_owned())?,
            language,
            rules,
            active: false,
        })
    }
}

impl Rule {
    /// Parses the rest of a `u:` line: `(<pattern>) $variable=$1`.
    fn parse(rest: &str) -> Option<Self> {
        let rest = rest.trim();
        let close = rest.find(')')?;
        let pattern = rest.get(1..close)?.trim();
        let output = rest.get(close + 1..)?.trim();
        let assignment = output
            .split_whitespace()
            .find(|word| word.starts_with('$') && word.contains('='))?;
        let variable = assignment
            .trim_start_matches('$')
            .split('=')
            .next()?
            .to_owned();
        let inputs = if pattern.contains('*') {
            None
        } else {
            let mut inputs = Vec::new();
            let mut current = None::<String>;
            for c in pattern.chars() {
                match (c, &mut current) {
                    ('"', None) => current = Some(String::new()),
                    ('"', Some(word)) => {
                        inputs.push(word.to_lowercase());
                        current = None;
                    }
                    (c, Some(word)) => word.push(c),
                    _ => {}
                }
            }
            if inputs.is_empty() {
                let words: Vec<String> = pattern
                    .split(|c: char| !c.is_alphanumeric() && c != ' ')
                    .map(|word| word.trim().to_lowercase())
                    .filter(|word| !word.is_empty())
                    .collect();
                if words.is_empty() {
                    None
                } else {
                    Some(words)
                }
            } else {
                Some(inputs)
            }
        };
        Some(Self { inputs, variable })
    }

    /// Returns true if the rule accepts the input.
    pub fn matches(&self, input: &str) -> bool {
        match &self.inputs {
            None => true,
            Some(inputs) => {
                let input = input.trim().to_lowercase();
                inputs.contains(&input)
            }
        }
    }
}

/// The state of the dialog engine. Cheap to clone: clones share the state.
#[derive(Clone)]
pub struct Dialog(Arc<Inner>);

struct Inner {
    context: Context,
    language: Mutex<String>,
    topics: Mutex<HashMap<String, Topic>>,
    subscribers: Mutex<HashSet<String>>,
    focus: Mutex<Option<String>>,
}

impl Dialog {
    /// Creates the dialog engine of a robot.
    pub fn new(context: &Context) -> Self {
        Self(Arc::new(Inner {
            context: context.clone(),
            language: Mutex::new("English".to_owned()),
            topics: Mutex::default(),
            subscribers: Mutex::default(),
            focus: Mutex::default(),
        }))
    }

    /// The current language.
    pub fn language(&self) -> String {
        lock(&self.0.language).clone()
    }

    /// The loaded topics.
    pub fn topics(&self) -> Vec<Topic> {
        let mut topics: Vec<Topic> = lock(&self.0.topics).values().cloned().collect();
        topics.sort_by(|a, b| a.name.cmp(&b.name));
        topics
    }

    /// Loads a topic from its content and returns its name.
    pub fn load_topic(&self, content: &str) -> qi::Result<String> {
        let topic = Topic::parse(content)
            .map_err(|err| error(format!("ALDialog::loadTopicContent\n\t{err}")))?;
        let name = topic.name.clone();
        self.0.context.logs.info(
            "ALDialog",
            format!("topic {name} loaded ({} rules)", topic.rules.len()),
        );
        lock(&self.0.topics).insert(name.clone(), topic);
        Ok(name)
    }

    /// Feeds an input to the activated topics: the variables of the matching rules are raised
    /// as memory events with the input. Returns the number of rules matched.
    pub fn input(&self, text: &str) -> usize {
        let memory = &self.0.context.memory;
        memory.insert("Dialog/LastInput", text);
        let variables: Vec<String> = lock(&self.0.topics)
            .values()
            .filter(|topic| topic.active)
            .flat_map(|topic| topic.rules.iter())
            .filter(|rule| rule.matches(text))
            .map(|rule| rule.variable.clone())
            .collect();
        self.0.context.logs.info(
            "ALDialog",
            format!("input \"{text}\" matched {} rules", variables.len()),
        );
        for variable in &variables {
            memory.raise(variable.clone(), text);
        }
        variables.len()
    }

    fn with_topic(&self, name: &str, method: &str, update: impl Fn(&mut Topic)) -> qi::Result<()> {
        let mut topics = lock(&self.0.topics);
        let topic = topics
            .get_mut(name)
            .ok_or_else(|| error(format!("ALDialog::{method}\n\tunknown topic: {name}")))?;
        update(topic);
        Ok(())
    }

    /// Builds the `ALDialog` service object.
    pub fn object(&self) -> AnyObject {
        let dialog = self.clone();
        let mut builder = ObjectBuilder::new();
        builder.set_description("ALDialog allows you to give your robot conversational skills by using a list of rules written in QiChat.");
        method!(builder, "setLanguage", [dialog], |language: String| {
            if !super::altexttospeech::LANGUAGES.contains(&language.as_str()) {
                return Err(error(format!(
                    "ALDialog::setLanguage\n\tunsupported language: {language}"
                )));
            }
            dialog
                .0
                .context
                .logs
                .info("ALDialog", format!("language set to {language}"));
            *lock(&dialog.0.language) = language;
            Ok(())
        });
        method!(builder, "getLanguage", [dialog], |(): ()| {
            Ok(dialog.language())
        });
        method!(builder, "loadTopicContent", [dialog], |content: String| {
            dialog.load_topic(&content)
        });
        method!(builder, "loadTopic", [dialog], |path: String| {
            let content = std::fs::read_to_string(&path).map_err(|err| {
                error(format!("ALDialog::loadTopic\n\tcannot read {path}: {err}"))
            })?;
            dialog.load_topic(&content)
        });
        method!(builder, "unloadTopic", [dialog], |name: String| {
            lock(&dialog.0.topics)
                .remove(&name)
                .map(|_| ())
                .ok_or_else(|| error(format!("ALDialog::unloadTopic\n\tunknown topic: {name}")))
        });
        method!(builder, "activateTopic", [dialog], |name: String| {
            dialog.with_topic(&name, "activateTopic", |topic| topic.active = true)
        });
        method!(builder, "deactivateTopic", [dialog], |name: String| {
            dialog.with_topic(&name, "deactivateTopic", |topic| topic.active = false)
        });
        method!(builder, "subscribe", [dialog], |name: String| {
            lock(&dialog.0.subscribers).insert(name);
            Ok(())
        });
        method!(builder, "unsubscribe", [dialog], |name: String| {
            lock(&dialog.0.subscribers).remove(&name);
            Ok(())
        });
        method!(builder, "setFocus", [dialog], |name: String| {
            dialog.with_topic(&name, "setFocus", |_| {})?;
            *lock(&dialog.0.focus) = Some(name);
            Ok(())
        });
        method!(builder, "getFocus", [dialog], |(): ()| {
            Ok(lock(&dialog.0.focus).clone().unwrap_or_default())
        });
        method!(builder, "getActivatedTopics", [dialog], |(): ()| {
            Ok(dialog
                .topics()
                .into_iter()
                .filter(|topic| topic.active)
                .map(|topic| topic.name)
                .collect::<Vec<String>>())
        });
        method!(builder, "getAllLoadedTopics", [dialog], |(): ()| {
            Ok(dialog
                .topics()
                .into_iter()
                .map(|topic| topic.name)
                .collect::<Vec<String>>())
        });
        method!(builder, "getLoadedTopics", [dialog], |language: String| {
            Ok(dialog
                .topics()
                .into_iter()
                .filter(|topic| topic.language.eq_ignore_ascii_case(&language))
                .map(|topic| topic.name)
                .collect::<Vec<String>>())
        });
        method!(builder, "forceInput", [dialog], |text: String| {
            dialog.input(&text);
            Ok(())
        });
        method!(builder, "forceOutput", [], |(): ()| { Ok(()) });
        method!(builder, "_recognize", [dialog], |text: String| {
            Ok(i32::try_from(dialog.input(&text)).unwrap_or(i32::MAX))
        });
        AnyObject::new(builder.build())
    }
}

impl std::fmt::Debug for Dialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dialog")
            .field("language", &self.language())
            .field("topics", &self.topics().len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_topics_are_understood() {
        let content = "topic: ~ros_abc ()\nlanguage: English\nu:(_[ \"yes\" \"no\" ]) $ros_abc/result=$1\nu:(_*) $ros_abc/result=$1\n";
        let topic = Topic::parse(content).unwrap();
        assert_eq!(topic.name, "ros_abc");
        assert_eq!(topic.language, "English");
        assert_eq!(topic.rules.len(), 2);
        assert_eq!(
            topic.rules[0].inputs,
            Some(vec!["yes".to_owned(), "no".to_owned()])
        );
        assert_eq!(topic.rules[0].variable, "ros_abc/result");
        assert!(topic.rules[0].matches("Yes"));
        assert!(!topic.rules[0].matches("maybe"));
        assert_eq!(topic.rules[1].inputs, None);
        assert!(topic.rules[1].matches("anything"));
        assert!(Topic::parse("language: English\n").is_err());
    }
}
