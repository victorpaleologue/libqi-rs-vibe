//! An example `qi` module written in Rust: the module `rustexample`.
//!
//! Its library, `librustexample.so`, is loaded by:
//!
//! - `qilaunch` (NAOqi 2.3 to 2.9, built with the `cpp` feature against the libqi of the robot):
//!   `qilaunch -n rustexample -o rustexample.RustGreeter`, or
//!   `qilaunch -n rustexample -f 'rustexample.say("Hello from Rust")'`;
//! - NAOqi (2.1 to 2.9), from its `autoload.ini`: the `RustGreeter` and `RustDcm` services are
//!   registered, and, built with the `naoqi` feature against the C++ SDK of NAOqi, the DCM plugin
//!   `RustEyes` runs in the loop of the DCM (see [`dcm`]);
//! - `qi-launch`, the Rust counterpart of `qilaunch`, with the same options.
//!
//! See `docs/modules.md` at the root of the repository.

pub mod dcm;

use qi::{module::Session, Property, Signal};

/// A service greeting people.
#[qi::object(case = "camelCase")]
pub trait Greeter {
    /// Greets someone: returns the greeting and notifies it.
    async fn greet(&self, name: String) -> qi::Result<String>;
    /// The names of the services of the space, as the module sees it.
    async fn services(&self) -> qi::Result<Vec<String>>;
    /// The version of the module.
    async fn version(&self) -> qi::Result<String>;
    /// Notified with each greeting.
    #[qi::signal]
    fn greeted(&self) -> &Signal<String>;
    /// The number of greetings so far.
    #[qi::property]
    fn greetings(&self) -> &Property<i32>;
}

/// The implementation of [`Greeter`].
pub struct RustGreeter {
    session: Session,
    greeted: Signal<String>,
    greetings: Property<i32>,
}

impl RustGreeter {
    pub fn new(session: Session) -> Self {
        Self {
            session,
            greeted: Signal::new(),
            greetings: Property::new(0),
        }
    }
}

#[qi::async_trait]
impl Greeter for RustGreeter {
    async fn greet(&self, name: String) -> qi::Result<String> {
        let greeting = rustexample::hello(name);
        self.greetings.set(self.greetings.get().await? + 1).await?;
        self.greeted.emit(greeting.clone());
        Ok(greeting)
    }

    async fn services(&self) -> qi::Result<Vec<String>> {
        rustexample::services(self.session.clone()).await
    }

    async fn version(&self) -> qi::Result<String> {
        Ok(env!("CARGO_PKG_VERSION").to_owned())
    }

    fn greeted(&self) -> &Signal<String> {
        &self.greeted
    }

    fn greetings(&self) -> &Property<i32> {
        &self.greetings
    }
}

/// An example module written in Rust.
#[qi::module(case = "camelCase")]
pub mod rustexample {
    use super::{
        dcm::{DcmReportObject, RustDcm, RustEyes},
        GreeterObject, RustGreeter,
    };
    use qi::{
        module::{naoqi::Naoqi, Session},
        ObjectExt,
    };

    /// Returns a greeting.
    pub fn hello(name: String) -> String {
        format!("Hello, {name}, from Rust!")
    }

    /// Adds two integers.
    pub fn add(a: i32, b: i32) -> i32 {
        a + b
    }

    /// Fails with the given message.
    pub fn fail(message: String) -> qi::Result<()> {
        Err(message.into())
    }

    /// Says a text with the `ALTextToSpeech` service of the space.
    pub async fn say(session: Session, text: String) -> qi::Result<()> {
        let tts = session.service("ALTextToSpeech").await?;
        tts.call("say", text).await
    }

    /// The names of the services of the space.
    pub async fn services(session: Session) -> qi::Result<Vec<String>> {
        use qi::service_directory::ServiceDirectory;
        let node = session.node().await?;
        let services = node.service_directory().services().await?;
        Ok(services.iter().map(|info| info.name().to_owned()).collect())
    }

    /// The `RustGreeter` service, registered when NAOqi loads the library.
    #[qi::factory(autoload)]
    pub fn rust_greeter(session: Session) -> qi::Result<GreeterObject<RustGreeter>> {
        Ok(GreeterObject::new(RustGreeter::new(session)))
    }

    /// The service reporting on the DCM plugin, registered when NAOqi loads the library.
    #[qi::factory(autoload)]
    pub fn rust_dcm() -> DcmReportObject<RustDcm> {
        DcmReportObject::new(RustDcm)
    }

    /// The DCM plugin: the ALModule `RustEyes`, created when NAOqi loads the library.
    #[qi::dcm(name = "RustEyes")]
    pub fn rust_eyes(naoqi: &Naoqi<'_>) -> qi::Result<RustEyes> {
        RustEyes::new(naoqi)
    }

    /// A greeter not registered by NAOqi, for `qilaunch -o rustexample.OtherGreeter`.
    #[qi::factory(name = "OtherGreeter")]
    pub async fn other_greeter(session: Session) -> qi::Result<GreeterObject<RustGreeter>> {
        Ok(GreeterObject::new(RustGreeter::new(session)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qi::ObjectExt;

    #[test]
    fn module_definition() {
        let module = &rustexample::__QI_MODULE;
        assert_eq!(module.name, "rustexample");
        assert_eq!(module.description, "An example module written in Rust.");
        let factories: Vec<_> = module
            .factories
            .iter()
            .map(|f| (f.name, f.autoload))
            .collect();
        assert_eq!(
            factories,
            [
                ("RustGreeter", true),
                ("RustDcm", true),
                ("OtherGreeter", false)
            ]
        );
        let plugins: Vec<_> = module.dcm_plugins.iter().map(|p| p.name).collect();
        assert_eq!(plugins, ["RustEyes"]);
    }

    #[tokio::test]
    async fn module_object() {
        let object = rustexample::__QI_MODULE.object();
        let names: Vec<_> = object
            .meta()
            .methods
            .values()
            .map(|m| m.name.clone())
            .collect();
        for name in [
            "hello",
            "add",
            "fail",
            "say",
            "services",
            "RustGreeter",
            "OtherGreeter",
        ] {
            assert!(names.contains(&name.to_owned()), "{name} in {names:?}");
        }
        let sum: i32 = object.call("add", (1, 2)).await.unwrap();
        assert_eq!(sum, 3);
        let greeting: String = object.call("hello", "Nao".to_owned()).await.unwrap();
        assert_eq!(greeting, "Hello, Nao, from Rust!");
        assert!(object
            .call::<(), _, _>("fail", "boom".to_owned())
            .await
            .is_err());
        let greeter: qi::AnyObject = object.call("RustGreeter", ()).await.unwrap();
        let greeting: String = greeter.call("greet", "Pepper".to_owned()).await.unwrap();
        assert_eq!(greeting, "Hello, Pepper, from Rust!");
        assert_eq!(greeter.property::<_, i32>("greetings").await.unwrap(), 1);
    }
}
