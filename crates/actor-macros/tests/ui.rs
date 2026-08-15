#[test]
fn message_derive_contract() {
    let tests = trybuild::TestCases::new();
    tests.pass("tests/ui/message/pass/*.rs");
    tests.compile_fail("tests/ui/message/fail/*.rs");
}

#[test]
fn actor_attribute_contract() {
    let tests = trybuild::TestCases::new();
    tests.pass("tests/ui/actor/pass/*.rs");
    tests.compile_fail("tests/ui/actor/fail/*.rs");
}
