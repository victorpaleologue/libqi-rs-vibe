//! Compile tests of `#[qi::module]`.

#[test]
fn module_members() {
    let t = trybuild::TestCases::new();
    t.pass("tests/module/pass_members.rs");
    t.compile_fail("tests/module/fail_cfg_member.rs");
    t.compile_fail("tests/module/fail_reserved_name.rs");
    t.compile_fail("tests/module/fail_autoload_parameters.rs");
    t.compile_fail("tests/module/fail_duplicate_names.rs");
}
