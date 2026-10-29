pub struct Plugin;
impl qi::module::naoqi::DcmPlugin for Plugin {}

#[qi::module]
pub mod duplicates {
    #[qi::factory(name = "Service")]
    pub fn service() -> qi::AnyObject {
        unimplemented!()
    }
    #[qi::dcm(name = "Service")]
    pub fn plugin(naoqi: &qi::module::naoqi::Naoqi<'_>) -> qi::Result<super::Plugin> {
        let _ = naoqi;
        Ok(super::Plugin)
    }
}

fn main() {}
