#[qi::module]
pub mod autoload {
    #[qi::factory(autoload)]
    pub fn service(start: i32) -> qi::AnyObject {
        let _ = start;
        unimplemented!()
    }
}

fn main() {}
