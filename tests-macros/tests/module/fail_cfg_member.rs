#[qi::module]
pub mod configured {
    #[cfg(feature = "something")]
    pub fn configured() {}
}

fn main() {}
