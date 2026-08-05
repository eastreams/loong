#[test]
fn public_type_contract() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/*.rs");
}
