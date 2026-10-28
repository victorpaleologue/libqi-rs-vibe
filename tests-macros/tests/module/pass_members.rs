//! The forms of module members that compile.

#[qi::object]
pub trait Counter {
    async fn count(&self) -> qi::Result<i32>;
}

pub struct One;

#[qi::async_trait]
impl Counter for One {
    async fn count(&self) -> qi::Result<i32> {
        Ok(1)
    }
}

pub struct Plugin;

impl qi::module::naoqi::DcmPlugin for Plugin {}

/// A module with every form of member.
#[qi::module(name = "some.module", case = "camelCase")]
pub mod members {
    use qi::module::{naoqi::Naoqi, Session};

    pub fn unit() {}
    pub fn plain(a: i32, b: String) -> String {
        format!("{a}{b}")
    }
    pub fn result(a: f64) -> qi::Result<f64> {
        Ok(a)
    }
    pub async fn asynchronous(session: Session, values: Vec<i32>) -> qi::Result<i32> {
        let _ = session;
        Ok(values.len() as i32)
    }
    #[qi::function(name = "renamed")]
    fn private_but_exported() -> bool {
        true
    }
    fn helper() -> i32 {
        2
    }
    pub fn uses_helper() -> i32 {
        helper()
    }
    #[qi::factory(autoload)]
    pub fn one(session: Session) -> super::CounterObject<super::One> {
        let _ = session;
        super::CounterObject::new(super::One)
    }
    #[qi::factory(name = "Parametrized")]
    pub async fn with_parameters(start: i32) -> qi::Result<qi::AnyObject> {
        let _ = start;
        Ok(super::CounterObject::new(super::One).into_any())
    }
    #[qi::dcm]
    pub fn plugin(naoqi: &Naoqi<'_>) -> qi::Result<super::Plugin> {
        let _ = naoqi;
        Ok(super::Plugin)
    }
}

fn main() {
    let module = &members::__QI_MODULE;
    assert_eq!(module.name, "some.module");
    assert_eq!(module.description, "A module with every form of member.");
    let factories: Vec<_> = module.factories.iter().map(|f| (f.name, f.autoload)).collect();
    assert_eq!(factories, [("One", true), ("Parametrized", false)]);
    assert_eq!(module.dcm_plugins.iter().map(|p| p.name).collect::<Vec<_>>(), ["Plugin"]);
    let object = module.object();
    let mut names: Vec<_> = object.meta().methods.values().map(|m| m.name.clone()).collect();
    names.sort();
    assert_eq!(
        names,
        ["One", "Parametrized", "asynchronous", "plain", "renamed", "result", "unit", "usesHelper"]
    );
}
